# mat

CLI som skriver ut dagens lunch på Mattias Mat-restaurangerna (Växthuset och
Orangeriet) i Skövde. Menyerna hämtas från
[mattiasmat.se](https://mattiasmat.se) och cachas lokalt per ISO-vecka så att
upprepade körningar är i princip omedelbara.

## Installation

```sh
cargo install matmat
```

Binären heter `mat` och installeras i `~/.cargo/bin/`.

## Användning

```sh
mat                 # dagens lunch på båda restaurangerna
mat vaxthuset       # bara Växthuset
mat orangeriet      # bara Orangeriet
mat --week          # hela veckans meny för båda
mat -w vaxthuset    # hela veckan för en restaurang
mat --refresh       # tvinga ny hämtning, ignorera cache
```

Veckodag beräknas i `Europe/Stockholm`, oavsett systemets lokala tidszon.

## Hur cachen fungerar

- Menyerna lagras i `~/Library/Caches/mat/` på macOS eller
  `$XDG_CACHE_HOME/mat/` (fallback `~/.cache/mat/`) på Linux.
- Cache-nyckeln är ISO-veckan (t.ex. `2026-W16`). När veckan rullar över missar
  cachen automatiskt och menyn hämtas på nytt.
- Vid en cache-hit skrivs resultatet direkt (~10 ms) och en detached
  bakgrundsprocess uppdaterar cachen tyst, så data hålls färskt utan att
  användaren väntar.
- `--refresh` kringgår cachen och gör synkron omhämtning.

## TLS-fallback

På vissa nät (t.ex. företagsproxyer med MITM-certifikat) kan certifikatet
avvisas av strikta validerare (`unknownissuer` via rustls, *Missing Authority
Key Identifier* i Python 3.14). Menyn är publik och icke-känslig, så binären
faller tyst tillbaka till oäkta verifiering för de kända signaturerna. Andra
SSL-fel skriver en varning till stderr innan retry.

## Bygga från källkod

```sh
git clone https://github.com/<USERNAME>/matmat
cd matmat
cargo build --release
./target/release/mat
```

Kräver Rust 1.80+ (för `std::sync::LazyLock`).

## Utgivning till crates.io

Nya versioner publiceras automatiskt via GitHub Actions när `version` i
`Cargo.toml` höjs och push sker till `main`:

```sh
# bumpa versionen:
sed -i '' 's/^version = ".*"/version = "0.2.0"/' Cargo.toml
git commit -am "release 0.2.0"
git push
```

Workflowen (`.github/workflows/publish.yml`) läser ut namn och version ur
`Cargo.toml`, frågar crates.io-API:t om versionen redan existerar, och kör
`cargo publish` bara om den är ny. Alla andra pushar till `main` blir no-op.

Tokenen skapas på
[crates.io/settings/tokens](https://crates.io/settings/tokens) (scope
`publish-update` räcker) och läggs till i repot som secret:

```sh
gh secret set CARGO_REGISTRY_TOKEN
```

## Licens

MIT — se [LICENSE](LICENSE).
