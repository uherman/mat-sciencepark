#!/usr/bin/env python3
"""mat — skriv ut dagens lunch på Mattias Mat-restaurangerna i Skövde.

Hämtar menyerna från mattiasmat.se och skriver ut dagens rätt
(Europe/Stockholm). Lägg till --week för hela veckan, eller ange en
restaurang (t.ex. `mat vaxthuset`) för att filtrera.
"""

from __future__ import annotations

import argparse
import html
import re
import ssl
import sys
import urllib.error
import urllib.request
from datetime import datetime
from zoneinfo import ZoneInfo

RESTAURANTS: dict[str, tuple[str, str]] = {
    # key: (display name, URL)
    "vaxthuset": ("Växthuset", "https://mattiasmat.se/restaurang/vaxthuset/"),
    "orangeriet": ("Orangeriet", "https://mattiasmat.se/restaurang/orangeriet/"),
}

DAYS = ["Måndag", "Tisdag", "Onsdag", "Torsdag", "Fredag", "Lördag", "Söndag"]


def fetch(url: str) -> str:
    req = urllib.request.Request(url, headers={"User-Agent": "mat-cli/1.0"})
    try:
        resp = urllib.request.urlopen(req, timeout=10)
    except urllib.error.URLError as e:
        # Corporate SSL proxies often present certificates that Python 3.14's
        # strict validation rejects (e.g., missing Authority Key Identifier).
        # The menu is public and non-sensitive, so retry unverified. Stay silent
        # for the known-benign AKI case; warn on other SSL errors so real
        # problems still surface.
        if isinstance(e.reason, ssl.SSLError):
            reason = str(e.reason)
            if "Missing Authority Key Identifier" not in reason:
                print(
                    f"mat: varning — SSL-verifiering misslyckades ({reason}); "
                    "försöker igen utan verifiering.",
                    file=sys.stderr,
                )
            ctx = ssl._create_unverified_context()
            resp = urllib.request.urlopen(req, timeout=10, context=ctx)
        else:
            raise
    with resp:
        charset = resp.headers.get_content_charset() or "utf-8"
        return resp.read().decode(charset, errors="replace")


def clean(fragment: str) -> str:
    text = re.sub(r"<[^>]+>", "", fragment)
    text = html.unescape(text)
    return " ".join(text.split()).rstrip(":").strip()


def parse_week(page: str) -> dict[str, list[str]]:
    """Return {day_name: [dish, ...]} for every day found in veckans-lunch."""
    block_match = re.search(
        r'<div class="veckans-lunch">(.*?)</div>', page, re.DOTALL
    )
    if not block_match:
        raise ValueError("hittade inte veckans-lunch-blocket på sidan")
    block = block_match.group(1)

    day_alt = "|".join(DAYS)
    pattern = re.compile(
        rf"<strong>({day_alt}):</strong>(.*?)(?=<strong>(?:{day_alt}):</strong>|\Z)",
        re.DOTALL,
    )
    result: dict[str, list[str]] = {}
    for m in pattern.finditer(block):
        day = m.group(1)
        # Each dish is wrapped in its own <p> tag. Split on <p> openings and
        # clean each chunk; pages with a single dish just yield one item.
        chunks = re.split(r"<p\b[^>]*>", m.group(2))
        dishes = [clean(c) for c in chunks]
        dishes = [d for d in dishes if d]
        if dishes:
            result[day] = dishes
    return result


def parse_week_number(page: str) -> str | None:
    m = re.search(r"Vecka\s+(\d+)", page)
    return m.group(1) if m else None


def today_day_name() -> str:
    return DAYS[datetime.now(ZoneInfo("Europe/Stockholm")).weekday()]


def render(name: str, url: str, show_week: bool, today: str) -> int:
    try:
        page = fetch(url)
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        print(f"{name}: kunde inte hämta menyn: {e}", file=sys.stderr)
        return 1

    try:
        week = parse_week(page)
    except ValueError as e:
        print(f"{name}: {e}", file=sys.stderr)
        return 1

    week_no = parse_week_number(page)
    suffix = f" v.{week_no}" if week_no else ""

    if show_week:
        print(f"{name} — hela veckan{suffix}")
        for day in DAYS:
            dishes = week.get(day) or ["—"]
            print(f"  {day}:")
            for dish in dishes:
                print(f"    • {dish}")
    else:
        dishes = week.get(today)
        print(f"{name} — {today}{suffix}")
        if dishes:
            for dish in dishes:
                print(f"  • {dish}")
        else:
            print("  Ingen meny hittades för idag.")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="mat",
        description="Visa dagens lunch på Mattias Mat-restaurangerna (Skövde).",
    )
    parser.add_argument(
        "restaurant",
        nargs="?",
        choices=sorted(RESTAURANTS.keys()),
        help="begränsa till en restaurang (default: alla)",
    )
    parser.add_argument(
        "-w", "--week", action="store_true", help="visa hela veckans meny"
    )
    args = parser.parse_args(argv)

    keys = [args.restaurant] if args.restaurant else list(RESTAURANTS.keys())
    today = today_day_name()

    rc = 0
    for i, key in enumerate(keys):
        name, url = RESTAURANTS[key]
        if i > 0:
            print()
        rc |= render(name, url, args.week, today)
    return rc


if __name__ == "__main__":
    sys.exit(main())
