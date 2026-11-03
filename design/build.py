#!/usr/bin/env python3
"""Expand design/src/boards/*.svg into standalone design/*.svg (light + dark).

Templates under design/src/ carry three kinds of shorthand:

  {{token}}        -> colour from design/src/tokens.json ({light, dark} pair)
  <!-- @icons -->  -> the shared <symbol> set from design/src/icons.svg
  literal #RRGGBB  -> recoloured for the dark variant via design/src/dark-literals.json

Every board produces two outputs:

  design/<name>.svg        light
  design/<name>.dark.svg   dark

Both are self-contained: no external references, no CSS custom properties, so
they open in a browser, Figma, or rsvg-convert unchanged.

Usage:  python design/build.py [--check]

--check exits non-zero if any output would change, which is what CI should run.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import xml.dom.minidom
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "src"
BOARDS = SRC / "boards"
TOKENS = SRC / "tokens.json"
DARK_LITERALS = SRC / "dark-literals.json"
ICONS = SRC / "icons.svg"

TOKEN_RE = re.compile(r"\{\{([a-z0-9-]+)\}\}")
HEX_RE = re.compile(r"#[0-9A-Fa-f]{6}\b")
USE_RE = re.compile(r'href="#([^"]+)"')
ID_RE = re.compile(r'\bid="([^"]+)"')

SCHEMES = ("light", "dark")


def load_tokens() -> dict[str, dict[str, str]]:
    raw = json.loads(TOKENS.read_text(encoding="utf-8"))
    out: dict[str, dict[str, str]] = {}
    for key, value in raw.items():
        if key.startswith("_"):
            continue
        if not isinstance(value, dict):
            raise SystemExit(f"{TOKENS}: token '{key}' must be {{light, dark}}")
        for scheme in SCHEMES:
            if scheme not in value:
                raise SystemExit(f"{TOKENS}: token '{key}' missing '{scheme}'")
        out[key] = value
    return out


def load_dark_literals() -> dict[str, str]:
    raw = json.loads(DARK_LITERALS.read_text(encoding="utf-8"))
    return {
        k.upper(): v
        for k, v in raw.items()
        if isinstance(v, str) and not k.startswith("_")
    }


def load_icons() -> str:
    raw = ICONS.read_text(encoding="utf-8")
    syms = re.findall(r"<symbol\b.*?</symbol>", raw, re.S)
    if not syms:
        raise SystemExit(f"{ICONS}: no <symbol> elements found")
    return "\n".join("    " + re.sub(r"\n\s*", "\n    ", s.strip()) for s in syms)


def expand(template: str, tokens: dict[str, dict[str, str]], icons: str, scheme: str) -> str:
    # Literal hexes are recoloured before token substitution, so the map only ever
    # touches colours the template hard-coded — never a value that came from tokens.json.
    text = template
    if scheme == "dark":
        lit = load_dark_literals()

        def recolour(m: re.Match[str]) -> str:
            return lit.get(m.group(0).upper(), m.group(0))

        text = HEX_RE.sub(recolour, text)

    missing: set[str] = set()

    def sub(m: re.Match[str]) -> str:
        name = m.group(1)
        if name not in tokens:
            missing.add(name)
            return m.group(0)
        return tokens[name][scheme]

    out = TOKEN_RE.sub(sub, text)
    if missing:
        raise SystemExit(f"unknown token(s): {', '.join(sorted(missing))}")

    if "<!-- @icons -->" in out:
        out = out.replace("<!-- @icons -->", icons)
    return out


def validate(name: str, text: str) -> None:
    try:
        xml.dom.minidom.parseString(text.encode("utf-8"))
    except Exception as exc:  # noqa: BLE001 - surface the parser message verbatim
        raise SystemExit(f"{name}: not well-formed XML: {exc}") from exc

    defined = set(ID_RE.findall(text))
    dangling = sorted({u for u in USE_RE.findall(text) if u not in defined})
    if dangling:
        raise SystemExit(f"{name}: <use> points at undefined id(s): {', '.join(dangling)}")

    if "{{" in text:
        raise SystemExit(f"{name}: unexpanded placeholder left in output")


def output_name(src: Path, scheme: str) -> str:
    return src.name if scheme == "light" else f"{src.stem}.dark.svg"


def build(check: bool) -> int:
    tokens = load_tokens()
    icons = load_icons()
    stale = 0

    boards = sorted(BOARDS.glob("*.svg"))
    if not boards:
        raise SystemExit(f"no board templates in {BOARDS}")

    for src in boards:
        template = src.read_text(encoding="utf-8")
        for scheme in SCHEMES:
            name = output_name(src, scheme)
            out_text = expand(template, tokens, icons, scheme)
            validate(name, out_text)
            dest = ROOT / name
            old = dest.read_text(encoding="utf-8") if dest.exists() else None
            if old == out_text:
                print(f"  unchanged  {name}")
                continue
            stale += 1
            if check:
                print(f"  STALE      {name}")
            else:
                dest.write_text(out_text, encoding="utf-8")
                print(f"  written    {name}")

    # stale dark files from a renamed board should not linger
    expected = {output_name(b, s) for b in boards for s in SCHEMES}
    for extra in ROOT.glob("*.dark.svg"):
        if extra.name not in expected:
            print(f"  orphan     {extra.name} (no matching template)")

    if check and stale:
        print(f"\n{stale} board(s) out of date — run: python design/build.py")
        return 1
    return 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if outputs are stale")
    sys.exit(build(ap.parse_args().check))
