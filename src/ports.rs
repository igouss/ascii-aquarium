//! What the app needs from the outside world.

use std::{io, time::Duration};

use crate::domain::{frame::Frame, input::Input};

/// Where frames are shown and input comes from: the terminal.
pub trait Screen {
    /// Columns, rows, and each cell's width and height in pixels.
    ///
    /// # Errors
    ///
    /// If the size can't be read.
    fn size(&mut self) -> io::Result<(usize, usize, f32, f32)>;

    /// # Errors
    ///
    /// If the frame can't be written.
    fn show(&mut self, frame: &Frame) -> io::Result<()>;

    /// Waits up to `timeout` for input, handing each to `input`.
    ///
    /// # Errors
    ///
    /// If input can't be read.
    fn wait(&mut self, timeout: Duration, input: &mut dyn FnMut(Input)) -> io::Result<()>;
}

/// Whether the screensaver should keep running: one of its windows has focus
/// and the session isn't locked.
pub trait Presence {
    fn should_stay(&mut self) -> bool;
}

impl<F: FnMut() -> bool> Presence for F {
    fn should_stay(&mut self) -> bool {
        self()
    }
}
