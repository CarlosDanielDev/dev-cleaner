#!/usr/bin/env python3
"""Generate the repository's logo art from the TUI's own logo grid.

    python3 docs/tools/logo_svg.py            # from the repo root

Reads `MASTER` out of `src/tui/logo.rs` (50 x 34 cells of `.`, `M` and `C`) and
writes three files into `docs/img/`:

- `logo.svg`            the logo alone, one <rect> per horizontal run, on a
                        transparent ground, so it sits on a light or a dark page
- `hero.svg`            the README banner: logo, gradient wordmark, tagline
- `social-preview.svg`  1280 x 640, for GitHub's social preview (which wants a
                        PNG or JPG: convert once and upload it in the repo's
                        Settings; no binary is committed)

The colours and the wordmark gradient are the TUI's, copied from
`src/tui/palette.rs` (`Theme::neon` and `Theme::brand`). The standard library
is all it needs. `tests/readme_claims.rs` asserts that `logo.svg` holds exactly
the cells of `MASTER`, so a change to the art that is not regenerated fails the
test.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAGENTA, CYAN = "#ff2e97", "#00e5ff"
INK = {"M": MAGENTA, "C": CYAN}
GROUND, TEXT, MUTED = "#0b0e1a", "#c8d3f5", "#8a98c4"
# Theme::brand: cyan -> violet -> magenta, in halves.
STOPS = ((0x00, 0xE5, 0xFF), (0xB4, 0x8C, 0xFF), (0xFF, 0x2E, 0x97))
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
NAME = "dev-cleaner"


def master():
    src = (ROOT / "src/tui/logo.rs").read_text()
    body = src.split("pub const MASTER: [&str; 34] = [", 1)[1].split("];", 1)[0]
    rows = re.findall(r'"([.MC]+)"', body)
    assert len(rows) == 34 and {len(r) for r in rows} == {50}, "MASTER changed shape"
    return rows


def runs(rows):
    """(x, y, width, ink) for every horizontal run of one ink."""
    for y, row in enumerate(rows):
        for m in re.finditer(r"M+|C+", row):
            yield m.start(), y, m.end() - m.start(), row[m.start()]


def rects(rows):
    return "\n".join(
        f'<rect x="{x}" y="{y}" width="{w}" height="1" fill="{INK[c]}"/>'
        for x, y, w, c in runs(rows)
    )


def brand(t):
    lo, hi, t = (STOPS[0], STOPS[1], t * 2) if t < 0.5 else (STOPS[1], STOPS[2], t * 2 - 1)
    return "#%02x%02x%02x" % tuple(round(a + (b - a) * t) for a, b in zip(lo, hi))


def wordmark(x, y, size):
    """The name a letter at a time, each in its place of the gradient; the
    hyphen is no step of it and is drawn quiet, as in the TUI."""
    cell = size * 0.6
    letters = len(NAME.replace("-", ""))
    out, at = [], 0
    for i, ch in enumerate(NAME):
        if ch == "-":
            fill = MUTED
        else:
            fill, at = brand(at / (letters - 1)), at + 1
        out.append(
            f'<text x="{x + cell * (i + 0.5):.1f}" y="{y}" fill="{fill}">{ch}</text>'
        )
    return (
        f'<g font-family="{MONO}" font-size="{size}" font-weight="700" '
        f'text-anchor="middle">' + "".join(out) + "</g>"
    )


def keycap(x, y, label, size):
    w = size * (0.62 * len(label) + 1.2)
    return (
        f'<rect x="{x}" y="{y - size * 1.05:.1f}" width="{w:.1f}" height="{size * 1.5:.1f}" '
        f'rx="{size * 0.25:.1f}" fill="{MAGENTA}"/>'
        f'<text x="{x + w / 2:.1f}" y="{y + size * 0.1:.1f}" text-anchor="middle" '
        f'font-family="{MONO}" font-size="{size}" font-weight="700" fill="{GROUND}">{label}</text>',
        w,
    )


def banner(rows, width, height, scale, left, top, name_size, tag_size):
    cap = name_size * 0.5
    caps, x = [], left + 50 * scale + 60
    base = top + 34 * scale
    for label, word in (("Space", "mark"), ("Enter", "next"), ("x", "hold to purge")):
        svg, w = keycap(x, base - 8, label, cap * 0.5)
        caps.append(svg)
        caps.append(
            f'<text x="{x + w + 8:.1f}" y="{base - 8:.1f}" font-family="{MONO}" '
            f'font-size="{cap * 0.5:.1f}" fill="{MUTED}">{word}</text>'
        )
        x += w + 8 + cap * 0.5 * (0.62 * len(word)) + 28
    tx = left + 50 * scale + 60
    # The tagline is wrapped by hand and every line carries a `textLength`, so a
    # font wider than the 0.6 em we assume can never run it off the canvas: the
    # renderer squeezes the spacing instead. A test checks that each line ends
    # inside the image.
    lines = (
        ("Map your developer folders.", TEXT),
        ("Measure what you can really get back.", TEXT),
        ("Deleting the wrong thing is structurally impossible.", MUTED),
    )
    ts = min(tag_size, (width - tx - 40) / (0.6 * max(len(t) for t, _ in lines)))
    y0 = top + name_size * 1.1 + ts * 1.9
    tagline = "".join(
        f'<text x="{tx}" y="{y0 + i * ts * 1.5:.1f}" fill="{fill}" '
        f'textLength="{0.6 * ts * len(text):.1f}" lengthAdjust="spacing">{text}</text>\n'
        for i, (text, fill) in enumerate(lines)
    )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="dev-cleaner: a pixel-art trash can with a code symbol on it">
<rect width="{width}" height="{height}" fill="{GROUND}"/>
<g transform="translate({left} {top}) scale({scale})" shape-rendering="crispEdges">
{rects(rows)}
</g>
{wordmark(tx, top + name_size * 1.1, name_size)}
<g font-family="{MONO}" font-size="{ts:.1f}">
{tagline}</g>
{"".join(caps)}
</svg>
"""


def main():
    rows = master()
    out = ROOT / "docs/img"
    out.mkdir(parents=True, exist_ok=True)
    (out / "logo.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="272" '
        'viewBox="0 0 50 34" shape-rendering="crispEdges" role="img" '
        'aria-label="dev-cleaner logo">\n' + rects(rows) + "\n</svg>\n"
    )
    (out / "hero.svg").write_text(banner(rows, 1000, 300, 6, 60, 48, 76, 19))
    (out / "social-preview.svg").write_text(banner(rows, 1280, 640, 8, 100, 184, 96, 24))
    for f in ("logo", "hero", "social-preview"):
        print(f"docs/img/{f}.svg", (out / f"{f}.svg").stat().st_size, "bytes")


if __name__ == "__main__":
    sys.exit(main())
