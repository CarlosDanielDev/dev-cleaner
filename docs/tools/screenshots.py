#!/usr/bin/env python3
"""Render real frames of the interface as SVG, for the README.

    python3 -m venv .venv && .venv/bin/pip install pyte
    .venv/bin/python docs/tools/screenshots.py target/debug/dev-cleaner WORKDIR

WORKDIR must not exist or must be empty. It gets:

- `home/`            an isolated HOME: the real config, history and Trash are
                     never read or written
- `home/projects/`   a synthetic tree of six small projects (about 270 MB of
                     real bytes at the default `--scale 0.5`)

The binary runs inside a pty (never tmux) at 100x30 with COLORTERM=truecolor,
is driven with the keys a person would press (Enter, Enter, Tab, `a`, Enter,
Enter) and is stopped on the confirmation screen without ever pressing the hold
key: nothing is purged. Each frame is read back through pyte and written to
`docs/img/<screen>.svg` as one <rect> per background run and one <text> per
run of glyphs, in the colours the terminal was given. No script, no raster.

Half blocks and braille, which fonts draw unevenly, are drawn as rectangles and
dots so the logo is the same art in every viewer.
"""
import argparse
import fcntl
import getpass
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
from html import escape
from pathlib import Path

import pyte

ROWS, COLS = 30, 100
CW, CH, FONT = 8.4, 17, 14
GROUND, TEXT = "#0b0e1a", "#c8d3f5"
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
OUT = Path(__file__).resolve().parents[1] / "img"

# name, files at the top, artifact dirs (relative dir, MB), days since last
# commit, what is left unsaved. Names are invented.
PROJECTS = [
    ("orbit-api", {"Cargo.toml": "[package]\nname='orbit-api'\n"}, [("target", 180)], 2, None),
    ("neon-web", {"package.json": "{}"}, [("node_modules", 90), (".next", 30)], 5, None),
    ("ledger-cli", {"requirements.txt": "rich\n"}, [(".venv", 60)], 90, None),
    ("pixel-game", {"Cargo.toml": "[package]\nname='pixel-game'\n"}, [("target", 120)], 420, None),
    ("dotfiles-old", {"package.json": "{}"}, [("node_modules", 40)], 500, "unpushed"),
    ("scratch-notes", {"package.json": "{}"}, [("node_modules", 15)], 380, "untracked"),
]


def git(cwd, env, *args):
    subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True)


def make_tree(root, scale):
    now = time.time()
    for name, files, artifacts, days, unsaved in PROJECTS:
        p = root / name
        (p / "src").mkdir(parents=True)
        (p / "src" / "main.txt").write_text("code\n")
        for f, body in files.items():
            (p / f).write_text(body)
        for d, mb in artifacts:
            (p / d).mkdir()
            with open(p / d / "blob.bin", "wb") as fh:
                for _ in range(max(1, int(mb * scale))):
                    fh.write(os.urandom(1024 * 1024))
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(now - days * 86400))
        env = {
            "PATH": os.environ["PATH"], "HOME": str(root.parent),
            "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null",
            "GIT_AUTHOR_NAME": "x", "GIT_AUTHOR_EMAIL": "x@example.invalid",
            "GIT_COMMITTER_NAME": "x", "GIT_COMMITTER_EMAIL": "x@example.invalid",
            "GIT_AUTHOR_DATE": stamp, "GIT_COMMITTER_DATE": stamp,
        }
        git(p, env, "init", "-q", "-b", "main")
        
        (p / ".gitignore").write_text("\n".join(d for d, _ in artifacts) + "\n")
        git(p, env, "add", "-A")
        git(p, env, "commit", "-q", "-m", "init")
        git(p, env, "remote", "add", "origin", f"https://example.invalid/{name}.git")
        git(p, env, "update-ref", "refs/remotes/origin/main", "HEAD")
        if unsaved == "unpushed":
            (p / "src" / "more.txt").write_text("more\n")
            git(p, env, "add", "-A")
            git(p, env, "commit", "-q", "-m", "work that exists nowhere else")
        if unsaved == "untracked":
            (p / "notes.txt").write_text("untracked\n")
        # The classifier reads file times too, .git's included: age them all.
        then = now - days * 86400
        for dirpath, dirs, names in os.walk(p):
            for n in names + dirs:
                os.utime(os.path.join(dirpath, n), (then, then))


def run(binary, home, root):
    """Drive the binary through the flow; return [(name, pyte screen)]."""
    env = {"HOME": str(home), "TERM": "xterm-256color", "COLORTERM": "truecolor",
           "PATH": "/usr/bin:/bin"}
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(binary, [binary, "tui", str(root)], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def settle(quiet=0.6, limit=20):
        last, end = time.time(), time.time() + limit
        while time.time() < end and time.time() - last < quiet:
            if select.select([fd], [], [], 0.1)[0]:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    return
                if data:
                    stream.feed(data)
                    # The interface repaints a 25-byte no-op ten times a
                    # second; only a real redraw restarts the quiet clock.
                    if len(data) > 64:
                        last = time.time()

    def frame(name):
        settle()
        return name, [[screen.buffer[y][x] for x in range(COLS)] for y in range(ROWS)]

    frames = []
    try:
        frames.append(frame("dashboard"))
        # Enter, Enter opens the candidates of the first project; Tab widens
        # them to every project, `a` marks what may be marked.
        for keys, name in ((b"\r", "projects"), (b"\r\t", None), (b"a", "candidates"),
                           (b"\r", "plan"), (b"\r", "confirm")):
            os.write(fd, keys)
            if name:
                frames.append(frame(name))
            else:
                settle()
    finally:
        # Never the hold key: nothing is purged. Our own child only; the
        # interface may sit on a signal while a screen is open, so insist.
        for sig in (signal.SIGTERM, signal.SIGKILL):
            os.kill(pid, sig)
            for _ in range(30):
                if os.waitpid(pid, os.WNOHANG)[0]:
                    sig = None
                    break
                time.sleep(0.1)
            if sig is None:
                break
    return frames


def scrub(grid, work):
    """Keep the machine's own path out of a screenshot: the dashboard prints
    the scanned root, cut short or whole, and that is the scratch directory."""
    parts = Path(work).parts[1:3]  # e.g. ("private", "tmp")
    pattern = re.compile("/" + "/".join(parts) + r"/\S*")
    for row in grid:
        text = "".join(c.data for c in row)
        for m in pattern.finditer(text):
            span = m.group()
            fresh = ("~/demo" + ("…" if span.endswith("…") else "")).ljust(len(span))
            for i, ch in enumerate(fresh):
                row[m.start() + i] = row[m.start() + i]._replace(data=ch)
    return grid


def colour(value, default):
    if value == "default":
        return default
    if len(value) == 6 and all(c in "0123456789abcdefABCDEF" for c in value):
        return "#" + value.lower()
    return default  # a named colour: only the truecolor look is captured


def braille(ch, x, y, fill):
    bits = ord(ch) - 0x2800
    dots = [(0, 0, 0), (0, 1, 1), (0, 2, 2), (1, 0, 3), (1, 1, 4), (1, 2, 5), (0, 3, 6), (1, 3, 7)]
    return "".join(
        f'<circle cx="{x + CW * (0.28 + 0.44 * dx):.1f}" cy="{y + CH * (0.125 + 0.25 * dy):.1f}" '
        f'r="{CW * 0.2:.1f}" fill="{fill}"/>'
        for dx, dy, bit in dots if bits >> bit & 1
    )


def to_svg(grid):
    bg, fg = [], []
    for r, row in enumerate(grid):
        y = r * CH
        # background runs
        c = 0
        while c < COLS:
            ch = row[c]
            back = colour(ch.bg, GROUND) if not ch.reverse else colour(ch.fg, TEXT)
            end = c
            while end + 1 < COLS and (
                colour(row[end + 1].bg, GROUND) if not row[end + 1].reverse
                else colour(row[end + 1].fg, TEXT)) == back:
                end += 1
            if back != GROUND:
                bg.append(f'<rect x="{c * CW:.1f}" y="{y}" width="{(end - c + 1) * CW:.1f}" '
                          f'height="{CH}" fill="{back}"/>')
            c = end + 1
        # glyph runs
        c = 0
        while c < COLS:
            ch = row[c]
            if ch.data.strip() == "":
                c += 1
                continue
            ink = colour(ch.fg, TEXT) if not ch.reverse else colour(ch.bg, GROUND)
            x = c * CW
            if "⠀" <= ch.data <= "⣿":
                fg.append(braille(ch.data, x, y, ink))
                c += 1
            elif ch.data in "▀▄█":
                lo, hi = {"▀": (0, 0.5), "▄": (0.5, 1), "█": (0, 1)}[ch.data]
                fg.append(f'<rect x="{x:.1f}" y="{y + lo * CH:.1f}" width="{CW:.1f}" '
                          f'height="{(hi - lo) * CH:.1f}" fill="{ink}"/>')
                c += 1
            else:
                end = c
                while end + 1 < COLS:
                    n = row[end + 1]
                    nink = colour(n.fg, TEXT) if not n.reverse else colour(n.bg, GROUND)
                    if (n.data.strip() == "" or n.data in "▀▄█" or "⠀" <= n.data <= "⣿"
                            or nink != ink or n.bold != ch.bold):
                        break
                    end += 1
                text = "".join(row[i].data for i in range(c, end + 1))
                weight = ' font-weight="700"' if ch.bold else ""
                fg.append(f'<text x="{x:.1f}" y="{y + CH * 0.78:.1f}" fill="{ink}"{weight} '
                          f'textLength="{(end - c + 1) * CW:.1f}" lengthAdjust="spacingAndGlyphs">'
                          f'{escape(text)}</text>')
                c = end + 1
    w, h = COLS * CW, ROWS * CH
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0f}" height="{h:.0f}" '
            f'viewBox="0 0 {w:.0f} {h:.0f}" role="img" aria-label="dev-cleaner terminal screenshot">\n'
            f'<rect width="{w:.0f}" height="{h:.0f}" rx="8" fill="{GROUND}"/>\n'
            f'<g>\n{chr(10).join(bg)}\n</g>\n'
            f'<g font-family="{MONO}" font-size="{FONT}" xml:space="preserve">\n{chr(10).join(fg)}\n</g>\n</svg>\n')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("workdir")
    ap.add_argument("--scale", type=float, default=0.5)
    a = ap.parse_args()
    work = Path(a.workdir).resolve()
    home = work / "home"
    root = home / "projects"
    root.mkdir(parents=True)
    make_tree(root, a.scale)
    cfg = home / ".config" / "dev-cleaner"
    cfg.mkdir(parents=True)
    (cfg / "config.toml").write_text(f'roots = ["{root}"]\ncaches = []\ndenylist = []\n')
    OUT.mkdir(parents=True, exist_ok=True)
    for name, grid in run(str(Path(a.binary).resolve()), home, root):
        svg = to_svg(scrub(grid, work))
        for leak in (str(work), getpass.getuser()):
            assert leak not in svg, f"{name}: the screenshot names {leak!r}"
        (OUT / f"{name}.svg").write_text(svg)
        print(f"docs/img/{name}.svg", len(svg), "bytes")


if __name__ == "__main__":
    sys.exit(main())
