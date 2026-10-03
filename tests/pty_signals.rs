//! A signal while a scan runs gives the terminal back.
//!
//! Driven through a real pty by `tests/pty/signal.py`, which spawns the binary
//! under an isolated HOME and signals only that process. A signal used to leave
//! the alternate screen up and the cursor hidden.

mod common;

use std::process::Command;

use common::Fixture;

fn python() -> Option<&'static str> {
    Command::new("python3")
        .arg("--version")
        .output()
        .ok()
        .map(|_| "python3")
}

fn run(signal: i32) -> serde_json::Value {
    let home = Fixture::new();
    let tree = Fixture::new();
    // Enough to be mid-walk when the first frame is seen, on any machine.
    for dir in 0..200 {
        for file in 0..100 {
            tree.file(&format!("p{dir}/f{file}"), b"x");
        }
        tree.file(&format!("p{dir}/package.json"), b"{}");
    }
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pty/signal.py");
    let out = Command::new(python().expect("python3 is needed for the pty tests"))
        .args([script, env!("CARGO_BIN_EXE_dev-cleaner")])
        .arg(home.root())
        .arg(tree.root())
        .arg(signal.to_string())
        .output()
        .expect("run the pty harness");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("the harness prints JSON")
}

#[test]
fn sigint_sigterm_and_sighup_each_restore_the_terminal_and_end_the_process() {
    for (signal, name, code) in [(2, "SIGINT", 130), (15, "SIGTERM", 143), (1, "SIGHUP", 129)] {
        let got = run(signal);
        assert_eq!(
            got["sent"], true,
            "{name}: the scan screen never appeared: {got}"
        );
        assert_eq!(got["left_alternate_screen"], true, "{name}: {got}");
        assert_eq!(got["showed_cursor"], true, "{name}: {got}");
        assert_eq!(got["exited"], true, "{name}: still running: {got}");
        assert_eq!(got["exit_code"], code, "{name}: {got}");
    }
}
