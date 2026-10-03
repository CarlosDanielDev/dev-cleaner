"""Run `dev-cleaner tui` in a pty, send it one signal while it scans, report
what the terminal was given back.

Usage: signal.py BIN HOME ROOT SIGNAL

The signal goes to the one process spawned here, never to a group. Prints one
line of JSON for the Rust test that runs this.
"""
import fcntl
import json
import os
import pty
import select
import struct
import sys
import termios
import time

binary, home, root, signum = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])

env = {"HOME": home, "TERM": "xterm-256color", "PATH": "/usr/bin:/bin"}
pid, fd = pty.fork()
if pid == 0:
    os.execve(binary, [binary, "tui", root], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 100, 0, 0))

seen = b""
sent = False
deadline = time.time() + 30
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.05)
    if ready:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        seen += chunk
    if not sent and b"Scanning" in seen:
        os.kill(pid, signum)
        sent = True

status = None
for _ in range(100):
    done, raw = os.waitpid(pid, os.WNOHANG)
    if done:
        status = raw
        break
    time.sleep(0.05)
if status is None:
    os.kill(pid, 9)
    os.waitpid(pid, 0)

print(
    json.dumps(
        {
            "sent": sent,
            "left_alternate_screen": b"\x1b[?1049l" in seen,
            "showed_cursor": b"\x1b[?25h" in seen,
            "exited": status is not None,
            "exit_code": os.WEXITSTATUS(status) if status is not None and os.WIFEXITED(status) else None,
            "killed_by": os.WTERMSIG(status) if status is not None and os.WIFSIGNALED(status) else None,
        }
    )
)
