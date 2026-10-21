//! Closing the terminal ends the screensaver quietly: no abort, and the run
//! log records why it stopped.

use std::{
    fs::{self, File},
    os::fd::OwnedFd,
    os::unix::process::ExitStatusExt,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

#[test]
fn a_closed_terminal_is_a_closed_window() {
    let master: OwnedFd = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let slave = ptsname(&master, Vec::new()).unwrap();
    let open = || {
        File::options()
            .read(true)
            .write(true)
            .open(slave.to_str().unwrap())
            .unwrap()
    };
    let log = std::env::temp_dir().join(format!("ascii-aquarium-hangup-{}", std::process::id()));

    // As in foot: a session of its own, with the terminal as its controlling
    // one, so closing the terminal hangs it up.
    let mut child = Command::new("setsid")
        .args([
            "--ctty",
            env!("CARGO_BIN_EXE_ascii-aquarium"),
            "koi",
            "--seed",
            "1",
            "--log",
        ])
        .arg(&log)
        .stdin(Stdio::from(open()))
        .stdout(Stdio::from(open()))
        .stderr(Stdio::from(open()))
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(1500));
    drop(master);
    let status = child.wait().unwrap();

    let entries = fs::read_to_string(&log).unwrap();
    fs::remove_file(&log).unwrap();
    assert_eq!(status.signal(), None, "killed by a signal: {status}");
    assert!(status.success(), "{status}");
    assert!(
        entries.lines().last().unwrap().ends_with("window closed"),
        "{entries}"
    );
}
