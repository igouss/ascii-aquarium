//! Dismissing one monitor's screensaver dismisses them all.

use std::{fs, process};

use rustix::process::{Pid, Signal, kill_process};

/// Sends SIGTERM to every other process running this same executable.
/// Matching the executable rather than the command line leaves alone
/// unrelated processes that merely mention the app id.
pub fn terminate_siblings() {
    let Ok(own_exe) = fs::read_link("/proc/self/exe") else {
        return;
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        return;
    };
    let own_pid = process::id();
    let siblings = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| pid != own_pid)
        .filter(|pid| fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| exe == own_exe));
    for pid in siblings.filter_map(|pid| Pid::from_raw(i32::try_from(pid).ok()?)) {
        let _ = kill_process(pid, Signal::TERM);
    }
}
