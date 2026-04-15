//! mat — skriv ut dagens lunch på Mattias Mat-restaurangerna i Skövde.
//!
//! Hämtar menyerna från mattiasmat.se och skriver ut dagens rätt
//! (Europe/Stockholm). Lägg till --week för hela veckan, eller ange en
//! restaurang (t.ex. `mat vaxthuset`) för att filtrera.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{Datelike, Utc};
use chrono_tz::Europe::Stockholm;
use clap::{Parser, ValueEnum};
use regex::Regex;
use serde::{Deserialize, Serialize};

const DAYS: [&str; 7] = [
    "Måndag", "Tisdag", "Onsdag", "Torsdag", "Fredag", "Lördag", "Söndag",
];

#[derive(Copy, Clone, ValueEnum)]
enum Restaurant {
    Vaxthuset,
    Orangeriet,
}

impl Restaurant {
    fn all() -> &'static [Restaurant] {
        &[Restaurant::Vaxthuset, Restaurant::Orangeriet]
    }

    fn display(self) -> &'static str {
        match self {
            Restaurant::Vaxthuset => "Växthuset",
            Restaurant::Orangeriet => "Orangeriet",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Restaurant::Vaxthuset => "https://mattiasmat.se/restaurang/vaxthuset/",
            Restaurant::Orangeriet => "https://mattiasmat.se/restaurang/orangeriet/",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Restaurant::Vaxthuset => "vaxthuset",
            Restaurant::Orangeriet => "orangeriet",
        }
    }
}

#[derive(Parser)]
#[command(
    name = "mat",
    about = "Visa dagens lunch på Mattias Mat-restaurangerna (Skövde)."
)]
struct Args {
    /// begränsa till en restaurang (default: alla)
    restaurant: Option<Restaurant>,
    /// visa hela veckans meny
    #[arg(short, long)]
    week: bool,
    /// tvinga ny hämtning och ignorera cache
    #[arg(long)]
    refresh: bool,
    /// Intern: hämtar och skriver cache, sedan avslut. Används av
    /// huvudprocessen för att refresha cache i bakgrunden.
    #[arg(long, hide = true)]
    background_refresh: bool,
}

static VECKANS_LUNCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<div class="veckans-lunch">(.*?)</div>"#).unwrap());

static DAY_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    let alt = DAYS.join("|");
    Regex::new(&format!(r"<strong>({alt}):</strong>")).unwrap()
});

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static P_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<p\b[^>]*>").unwrap());
static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static VECKA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Vecka\s+(\d+)").unwrap());

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    iso_week: String,
    week_no: Option<String>,
    week: HashMap<String, Vec<String>>,
    fetched_at: String,
}

fn current_iso_week() -> String {
    let now = Utc::now().with_timezone(&Stockholm);
    let iso = now.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

fn cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/mat"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(base.join("mat"))
    }
}

fn cache_path(key: &str) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(format!("{key}.json")))
}

fn read_cache(r: Restaurant) -> Option<CacheEntry> {
    let path = cache_path(r.key())?;
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(r: Restaurant, entry: &CacheEntry) {
    let Some(path) = cache_path(r.key()) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec_pretty(entry) else {
        return;
    };
    // Atomär skrivning: skriv till tmp-fil + rename, så parallella
    // bakgrundsrefresher inte kan lämna en halvskriven cachefil.
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &json).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

fn fetch_and_save(r: Restaurant) -> Result<CacheEntry, String> {
    let page = fetch(r.url())?;
    let week = parse_week(&page)?;
    let week_no = parse_week_number(&page);
    let entry = CacheEntry {
        iso_week: current_iso_week(),
        week_no,
        week,
        fetched_at: Utc::now().with_timezone(&Stockholm).to_rfc3339(),
    };
    write_cache(r, &entry);
    Ok(entry)
}

fn spawn_background_refresh(r: Restaurant) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .arg("--background-refresh")
        .arg(r.key())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn load_or_fetch(r: Restaurant, force_refresh: bool) -> Result<CacheEntry, String> {
    if !force_refresh {
        if let Some(cached) = read_cache(r) {
            if cached.iso_week == current_iso_week() {
                spawn_background_refresh(r);
                return Ok(cached);
            }
        }
    }
    fetch_and_save(r)
}

fn fetch(url: &str) -> Result<String, String> {
    let ua = "mat-cli/1.0";
    let timeout = Duration::from_secs(10);
    let request = |accept_invalid: bool| -> Result<String, reqwest::Error> {
        reqwest::blocking::Client::builder()
            .user_agent(ua)
            .timeout(timeout)
            .danger_accept_invalid_certs(accept_invalid)
            .build()?
            .get(url)
            .send()?
            .error_for_status()?
            .text()
    };

    match request(false) {
        Ok(body) => Ok(body),
        Err(e) if looks_like_tls_error(&e) => {
            // Företags-SSL-proxyer presenterar certifikat som strikta
            // validerare avvisar ("unknownissuer" via rustls/webpki).
            // Menyn är publik och icke-känslig — prova igen utan verifiering.
            // Var tyst för de kända signaturerna; varna för andra SSL-fel så
            // att verkliga problem märks.
            if !looks_like_known_corporate_cert(&e) {
                eprintln!(
                    "mat: varning — SSL-verifiering misslyckades ({e}); försöker igen utan verifiering."
                );
            }
            request(true).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn error_chain(err: &reqwest::Error) -> String {
    let mut s = format!("{err}");
    let mut cur: Option<&(dyn StdError + 'static)> = err.source();
    while let Some(src) = cur {
        s.push_str(" | ");
        s.push_str(&src.to_string());
        cur = src.source();
    }
    s.to_lowercase()
}

fn looks_like_tls_error(err: &reqwest::Error) -> bool {
    let s = error_chain(err);
    s.contains("tls")
        || s.contains("ssl")
        || s.contains("certificate")
        || s.contains("webpki")
        || s.contains("rustls")
}

fn looks_like_known_corporate_cert(err: &reqwest::Error) -> bool {
    let s = error_chain(err);
    s.contains("authority key identifier")
        || s.contains("authoritykeyidentifier")
        || s.contains("unknownissuer")
        || s.contains("unknown issuer")
}

fn clean(fragment: &str) -> String {
    let stripped = TAG.replace_all(fragment, "");
    let decoded = html_escape::decode_html_entities(stripped.as_ref());
    let collapsed = WS.replace_all(decoded.trim(), " ");
    collapsed.trim_end_matches(':').trim().to_string()
}

fn parse_week(page: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let block = VECKANS_LUNCH
        .captures(page)
        .ok_or_else(|| "hittade inte veckans-lunch-blocket på sidan".to_string())?
        .get(1)
        .unwrap()
        .as_str();

    // Samla dag-markörers positioner och namn. Regex-crate stöder inte
    // lookahead, så vi iterar markörer parvis och tar innehållet mellan dem.
    let markers: Vec<(usize, usize, String)> = DAY_MARKER
        .captures_iter(block)
        .map(|cap| {
            let whole = cap.get(0).unwrap();
            let day = cap.get(1).unwrap().as_str().to_string();
            (whole.start(), whole.end(), day)
        })
        .collect();

    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    for (i, (_start, end, day)) in markers.iter().enumerate() {
        let next_start = markers
            .get(i + 1)
            .map(|(s, _, _)| *s)
            .unwrap_or_else(|| block.len());
        let body = &block[*end..next_start];
        let dishes: Vec<String> = P_OPEN
            .split(body)
            .map(clean)
            .filter(|d| !d.is_empty())
            .collect();
        if !dishes.is_empty() {
            result.insert(day.clone(), dishes);
        }
    }
    Ok(result)
}

fn parse_week_number(page: &str) -> Option<String> {
    VECKA
        .captures(page)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn today_day_name() -> &'static str {
    let idx = Utc::now()
        .with_timezone(&Stockholm)
        .weekday()
        .num_days_from_monday() as usize;
    DAYS[idx]
}

fn render(r: Restaurant, show_week: bool, today: &str, force_refresh: bool) -> i32 {
    let name = r.display();

    let entry = match load_or_fetch(r, force_refresh) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{name}: kunde inte hämta menyn: {e}");
            return 1;
        }
    };

    let suffix = match &entry.week_no {
        Some(n) => format!(" v.{n}"),
        None => String::new(),
    };

    if show_week {
        println!("{name} — hela veckan{suffix}");
        for day in DAYS.iter() {
            println!("  {day}:");
            match entry.week.get(*day) {
                Some(dishes) => {
                    for dish in dishes {
                        println!("    • {dish}");
                    }
                }
                None => println!("    • —"),
            }
        }
    } else {
        println!("{name} — {today}{suffix}");
        match entry.week.get(today) {
            Some(dishes) => {
                for dish in dishes {
                    println!("  • {dish}");
                }
            }
            None => println!("  Ingen meny hittades för idag."),
        }
    }
    0
}

fn main() -> ExitCode {
    let args = Args::parse();

    // Bakgrundsrefresh-läge: hämta + spara, inga utskrifter, avsluta tyst.
    if args.background_refresh {
        let targets: Vec<Restaurant> = match args.restaurant {
            Some(r) => vec![r],
            None => Restaurant::all().to_vec(),
        };
        for r in targets {
            let _ = fetch_and_save(r);
        }
        return ExitCode::SUCCESS;
    }

    let keys: Vec<Restaurant> = match args.restaurant {
        Some(r) => vec![r],
        None => Restaurant::all().to_vec(),
    };
    let today = today_day_name();

    let mut rc: i32 = 0;
    for (i, r) in keys.iter().enumerate() {
        if i > 0 {
            println!();
        }
        rc |= render(*r, args.week, today, args.refresh);
    }

    if rc == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
