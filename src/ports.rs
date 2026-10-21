//! What the app needs from the outside world.

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
