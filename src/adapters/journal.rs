//! The run log the shader screensaver keeps: the rotation reads its
//! "showing" lines to know what played when.

use std::{
    fs::OpenOptions,
    io::{self, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Local;

use crate::domain::input::StopReason;

#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }

    /// `entry` is the rotation's entry for this scene, named as it lists it.
    ///
    /// # Errors
    ///
    /// If the log can't be appended to.
    pub fn showing(&self, entry: &Path) -> io::Result<()> {
        self.append(&format!("showing {} (ascii format)", entry.display()))
    }

    /// # Errors
    ///
    /// If the log can't be appended to.
    pub fn stopped(&self, after: Duration, reason: StopReason) -> io::Result<()> {
        self.append(&format!(
            "stopped after {:.0}s: {reason}",
            after.as_secs_f64()
        ))
    }

    fn append(&self, entry: &str) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{} {entry}", Local::now().format("%F %T"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_read_like_the_shader_screensavers() {
        let path =
            std::env::temp_dir().join(format!("ascii-aquarium-journal-{}", std::process::id()));
        let journal = Journal::new(&path);
        journal.showing(Path::new("/shaders/sharks.ascii")).unwrap();
        journal
            .stopped(Duration::from_millis(61_600), StopReason::MouseMoved)
            .unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let lines: Vec<&str> = log.lines().map(|l| &l[20..]).collect();
        assert_eq!(
            lines,
            [
                "showing /shaders/sharks.ascii (ascii format)",
                "stopped after 62s: mouse moved"
            ]
        );
        // The timestamp the rotation parses: "YYYY-MM-DD HH:MM:SS ".
        assert!(
            log.as_bytes()[4] == b'-' && log.as_bytes()[13] == b':' && log.as_bytes()[19] == b' '
        );
    }
}
