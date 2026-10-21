//! One screensaver session, driven by its host: frames on the host's clock,
//! input that may dismiss it, and a watch for focus loss.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use crate::{
    domain::{
        focus::FocusWatch,
        frame::{Color, Frame, Grid},
        input::{Dismissal, Input, StopReason},
        scene::{Kind, STEP, Scene},
    },
    ports::Presence,
};

/// At most this many simulation steps per frame; a slower machine runs slow
/// rather than spiral.
const MAX_STEPS: u32 = 4;
/// The longest frame time the wake spreads by.
const MAX_DT: f32 = 0.1;

/// The scene on screen, and what the input so far makes of dismissing it.
/// Times are since the session started.
#[derive(Debug)]
pub struct Session {
    kind: Kind,
    rng: fastrand::Rng,
    scene: Option<Scene>,
    frame: Frame,
    dismissal: Dismissal,
    last: Duration,
    behind: f32,
}

impl Session {
    #[must_use]
    pub fn new(kind: Kind, seed: u64) -> Self {
        Self {
            kind,
            rng: fastrand::Rng::with_seed(seed),
            scene: None,
            frame: Frame::new(0, 0, Color::default()),
            dismissal: Dismissal::default(),
            last: Duration::ZERO,
            behind: 0.0,
        }
    }

    /// Catches the simulation up to `now` on `grid` and draws it. A new grid
    /// starts the scene over: its pond is sized to the screen.
    pub fn advance(&mut self, grid: Grid, now: Duration) -> &Frame {
        if self.scene.as_ref().is_none_or(|s| s.grid() != grid) {
            self.scene = Some(Scene::new(&self.kind, grid, self.rng.fork()));
            self.frame = Frame::new(grid.cols, grid.rows, Color::default());
        }
        let Some(scene) = self.scene.as_mut() else {
            unreachable!("just made")
        };
        let dt = now.saturating_sub(self.last).as_secs_f32().min(MAX_DT);
        self.last = now;
        self.behind += dt;
        let mut steps = 0;
        while self.behind >= STEP && steps < MAX_STEPS {
            scene.step();
            self.behind -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS {
            self.behind = 0.0;
        }
        scene.render(&mut self.frame, dt);
        &self.frame
    }

    /// Whether `input` at `now` dismisses the screensaver, and why.
    pub fn input(&mut self, input: Input, now: Duration) -> Option<StopReason> {
        self.dismissal.judge(input, now)
    }
}

/// Asks `presence` whether the screensaver should stay, with a `poll`
/// between asks, until `stop` is set. Sets `stop` itself once the screensaver
/// has been absent for the grace period, and then returns true.
pub fn watch(
    mut presence: impl Presence,
    stop: &AtomicBool,
    now: impl Fn() -> Duration,
    poll: impl Fn(),
) -> bool {
    let mut focus = FocusWatch::default();
    while !stop.load(Ordering::Relaxed) {
        if focus.observe(presence.should_stay(), now()) {
            stop.store(true, Ordering::Relaxed);
            return true;
        }
        poll();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{focus::GRACE, shark::DEFAULT_SHARKS};
    use std::{cell::Cell, sync::atomic::AtomicU64};

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn sharks() -> Session {
        Session::new(Kind::Sharks(DEFAULT_SHARKS.to_vec()), 1)
    }

    const GRID: Grid = Grid {
        cols: 80,
        rows: 24,
        cw: 13.0,
        ch: 19.0,
    };

    #[test]
    fn frames_follow_the_grid() {
        let mut session = sharks();
        assert_eq!(session.advance(GRID, ms(33)).cells.len(), 80 * 24);
        let smaller = Grid {
            cols: 40,
            rows: 12,
            ..GRID
        };
        let frame = session.advance(smaller, ms(66));
        assert_eq!((frame.cols, frame.rows), (40, 12));
    }

    #[test]
    fn input_dismisses_once_armed() {
        let mut session = sharks();
        session.advance(GRID, ms(33));
        assert_eq!(session.input(Input::Key, ms(100)), None);
        assert_eq!(
            session.input(Input::Key, ms(2000)),
            Some(StopReason::KeyPress)
        );
    }

    #[test]
    fn a_stall_costs_at_most_a_few_steps() {
        let mut slow = sharks();
        slow.advance(GRID, ms(0));
        // A ten-second hitch is a tenth of a second of simulation, at most.
        slow.advance(GRID, ms(10_000));
        assert!(slow.behind < STEP, "{}", slow.behind);
    }

    /// A clock that only moves when the watch polls.
    #[derive(Default)]
    struct Clock(AtomicU64);

    impl Clock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::Relaxed))
        }

        fn advance(&self, by: u64) {
            let now = self.0.fetch_add(by, Ordering::Relaxed);
            assert!(now <= 120_000, "still watching after two minutes");
        }
    }

    #[test]
    fn losing_focus_for_the_grace_period_stops_it() {
        let clock = Clock::default();
        let stop = AtomicBool::new(false);
        let gone = watch(|| false, &stop, || clock.now(), || clock.advance(100));
        assert!(gone && stop.load(Ordering::Relaxed));
        assert!(clock.now() >= GRACE && clock.now() < GRACE + ms(200));
    }

    #[test]
    fn keeping_focus_keeps_the_watch_going_until_stopped() {
        let clock = Clock::default();
        let stop = AtomicBool::new(false);
        let asked = Cell::new(0);
        let gone = watch(
            || {
                asked.set(asked.get() + 1);
                true
            },
            &stop,
            || clock.now(),
            || {
                clock.advance(100);
                if clock.now() > ms(5000) {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(!gone);
        assert!(asked.get() > 40);
    }
}
