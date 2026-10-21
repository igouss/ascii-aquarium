//! Hyprland and Omarchy: whether the screensaver should keep running, and
//! hiding the pointer while it does.

use std::process::{Command, Stdio};

use crate::ports::Presence;

/// Same rule as Omarchy's terminal screensaver: keep running only while a
/// screensaver window is focused (any monitor's instance counts) and the
/// session isn't locked.
#[derive(Debug, Clone)]
pub struct Hyprland {
    app_id: String,
}

impl Hyprland {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
        }
    }
}

impl Presence for Hyprland {
    fn should_stay(&mut self) -> bool {
        !output_of("omarchy-shell", &["lock", "isLocked"]).contains("true")
            && is_active(&output_of("hyprctl", &["activewindow", "-j"]), &self.app_id)
    }
}

/// Hides the mouse pointer, everywhere, until dropped: the terminal the
/// screensaver runs in would otherwise show it.
#[derive(Debug)]
pub struct HiddenPointer(());

impl HiddenPointer {
    #[must_use]
    pub fn new() -> Self {
        set_pointer_invisible(true);
        Self(())
    }
}

impl Drop for HiddenPointer {
    fn drop(&mut self) {
        set_pointer_invisible(false);
    }
}

/// Through Hyprland's Lua config API where it has one, else the old keyword.
fn set_pointer_invisible(invisible: bool) {
    let lua = format!("hl.config({{ cursor = {{ invisible = {invisible} }} }})");
    if !succeeds("hyprctl", &["eval", &lua]) {
        succeeds(
            "hyprctl",
            &["keyword", "cursor:invisible", &invisible.to_string()],
        );
    }
}

/// Whether `hyprctl activewindow -j` output describes a window of class `app_id`.
fn is_active(active_window: &str, app_id: &str) -> bool {
    active_window.contains(&format!("\"class\": \"{app_id}\""))
}

/// A command's stdout, or nothing if it can't be run.
fn output_of(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

/// Whether a command runs and succeeds.
fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: &str = r#"{
    "address": "0x55d1",
    "class": "org.omarchy.screensaver",
    "title": "foot"
}"#;

    #[test]
    fn the_active_window_is_matched_by_class() {
        assert!(is_active(ACTIVE, "org.omarchy.screensaver"));
        assert!(!is_active(ACTIVE, "org.omarchy"));
        assert!(!is_active(
            &ACTIVE.replace("class", "initialClass"),
            "org.omarchy.screensaver"
        ));
        assert!(!is_active("{}", "org.omarchy.screensaver"));
    }

    #[test]
    fn commands_are_read_for_their_output() {
        assert_eq!(output_of("sh", &["-c", "echo out; echo err >&2"]), "out\n");
        assert_eq!(output_of("/nonexistent/command", &[]), "");
    }

    #[test]
    fn commands_succeed_only_when_they_run_and_exit_zero() {
        assert!(succeeds("sh", &["-c", "echo quiet"]));
        assert!(!succeeds("sh", &["-c", "exit 3"]));
        assert!(!succeeds("/nonexistent/command", &[]));
    }
}
