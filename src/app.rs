//! One screensaver session: the scene at a capped frame rate, input that may
//! dismiss it, and a watch for focus loss.

use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use crate::{
    domain::{
        focus::FocusWatch,
        frame::{Color, Frame, Grid},
        input::{Dismissal, StopReason},
        scene::{Kind, STEP, Scene},
    },
    ports::{Presence, Screen},
};

/// At most this many simulation steps per frame; a slower machine runs slow
/// rather than spiral.
const MAX_STEPS: u32 = 4;
/// The longest frame time the wake spreads by.
const MAX_DT: f32 = 0.1;

/// How the session runs.
#[derive(Debug, Clone)]
pub struct Settings {
    pub kind: Kind,
    /// Frames per second; the simulation always steps at 60.
    pub fps: u32,
    pub seed: u64,
}

/// Shows the scene until an input dismisses it, `stop` is set (by a signal),
/// or, given a `presence` to watch, the screensaver loses focus. `now` is
/// the time since the session started; `poll` pauses between presence
/// checks. Returns why it stopped.
///
/// # Errors
///
/// If the screen fails.
pub fn run<N, P>(
    screen: &mut impl Screen,
    settings: &Settings,
    presence: Option<impl Presence + Send>,
    stop: &AtomicBool,
    now: N,
    poll: P,
) -> io::Result<StopReason>
where
    N: Fn() -> Duration + Sync,
    P: Fn() + Sync,
{
    let (now, poll) = (&now, &poll);
    thread::scope(|scope| {
        let watcher =
            presence.map(|presence| scope.spawn(move || watch(presence, stop, now, poll)));
        let shown = match show(screen, settings, stop, now) {
            // The screen reports a terminal that is gone this way.
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(StopReason::WindowClosed),
            shown => shown,
        };
        // Ends the watch, if the show ended some other way.
        stop.store(true, Ordering::Relaxed);
        let focus_left = watcher.is_some_and(|watcher| watcher.join().unwrap_or(false));
        shown.map(|reason| {
            if focus_left {
                StopReason::FocusLeft
            } else {
                reason
            }
        })
    })
}

fn show(
    screen: &mut impl Screen,
    settings: &Settings,
    stop: &AtomicBool,
    now: impl Fn() -> Duration,
) -> io::Result<StopReason> {
    let frame_time = Duration::from_secs(1) / settings.fps.max(1);
    let mut rng = fastrand::Rng::with_seed(settings.seed);
    let mut scene: Option<Scene> = None;
    let mut frame = Frame::new(0, 0, Color::default());
    let mut dismissal = Dismissal::default();
    let mut last = now();
    let mut behind = 0.0f32;
    loop {
        let (cols, rows, cw, ch) = screen.size()?;
        let grid = Grid { cols, rows, cw, ch };
        // A new size starts the scene over: its pond is sized to the screen.
        if scene.as_ref().is_none_or(|s| s.grid() != grid) {
            scene = Some(Scene::new(&settings.kind, grid, rng.fork()));
            frame = Frame::new(cols, rows, Color::default());
        }
        let Some(scene) = scene.as_mut() else {
            unreachable!("just made")
        };

        let started = now();
        let dt = started.saturating_sub(last).as_secs_f32().min(MAX_DT);
        last = started;
        behind += dt;
        let mut steps = 0;
        while behind >= STEP && steps < MAX_STEPS {
            scene.step();
            behind -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS {
            behind = 0.0;
        }
        scene.render(&mut frame, dt);
        screen.show(&frame)?;

        let mut reason = None;
        loop {
            let left = frame_time.saturating_sub(now().saturating_sub(started));
            if stop.load(Ordering::Relaxed) {
                return Ok(reason.unwrap_or(StopReason::WindowClosed));
            }
            if left.is_zero() || reason.is_some() {
                break;
            }
            screen.wait(left, &mut |input| {
                reason = reason.or_else(|| dismissal.judge(input, now()));
            })?;
        }
        if let Some(reason) = reason {
            return Ok(reason);
        }
    }
}

/// Asks `presence` whether the screensaver should stay, with a `poll`
/// between asks, until `stop` is set. Sets `stop` itself once the screensaver
/// has been absent for the grace period, and then returns true.
fn watch(
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
    use crate::domain::{focus::GRACE, input::Input};
    use std::{cell::Cell, sync::atomic::AtomicU64};

    /// A clock that only moves when the screen waits or the watch polls.
    #[derive(Default)]
    struct Clock(AtomicU64);

    impl Clock {
        fn now(&self) -> Duration {
            Duration::from_nanos(self.0.load(Ordering::Relaxed))
        }

        fn advance(&self, by: Duration) {
            let now = self
                .0
                .fetch_add(u64::try_from(by.as_nanos()).unwrap(), Ordering::Relaxed);
            assert!(now <= 120_000_000_000, "still running after two minutes");
        }
    }

    /// An 80x24 terminal that feeds `inputs` at the given times.
    struct FakeScreen<'a> {
        clock: &'a Clock,
        inputs: Vec<(Duration, Input)>,
        frames: Cell<u32>,
        size: (usize, usize),
    }

    impl Screen for FakeScreen<'_> {
        fn size(&mut self) -> io::Result<(usize, usize, f32, f32)> {
            Ok((self.size.0, self.size.1, 13.0, 19.0))
        }

        fn show(&mut self, frame: &Frame) -> io::Result<()> {
            assert_eq!(frame.cells.len(), self.size.0 * self.size.1);
            self.frames.set(self.frames.get() + 1);
            Ok(())
        }

        fn wait(&mut self, timeout: Duration, input: &mut dyn FnMut(Input)) -> io::Result<()> {
            self.clock.advance(timeout);
            let now = self.clock.now();
            while let Some(&(at, next)) = self.inputs.first().filter(|(at, _)| *at <= now) {
                let _ = at;
                self.inputs.remove(0);
                input(next);
            }
            Ok(())
        }
    }

    fn settings() -> Settings {
        Settings {
            kind: Kind::Sharks(crate::domain::shark::DEFAULT_SHARKS.to_vec()),
            fps: 30,
            seed: 1,
        }
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_key_after_the_arm_time_dismisses_at_the_frame_rate() {
        let clock = Clock::default();
        let mut screen = FakeScreen {
            clock: &clock,
            inputs: vec![(ms(100), Input::Key), (ms(2000), Input::Key)],
            frames: Cell::new(0),
            size: (80, 24),
        };
        let none: Option<fn() -> bool> = None;
        let stop = AtomicBool::new(false);
        let reason = run(&mut screen, &settings(), none, &stop, || clock.now(), || {}).unwrap();
        assert_eq!(reason, StopReason::KeyPress);
        assert!(clock.now() >= ms(2000) && clock.now() < ms(2034));
        // 30 fps for two seconds.
        assert!(
            (59..=61).contains(&screen.frames.get()),
            "{} frames",
            screen.frames.get()
        );
    }

    #[test]
    fn a_signal_stops_the_show() {
        let clock = Clock::default();
        let mut screen = FakeScreen {
            clock: &clock,
            inputs: vec![],
            frames: Cell::new(0),
            size: (40, 12),
        };
        let stop = AtomicBool::new(true);
        let none: Option<fn() -> bool> = None;
        assert_eq!(
            run(&mut screen, &settings(), none, &stop, || clock.now(), || {}).unwrap(),
            StopReason::WindowClosed
        );
    }

    #[test]
    fn losing_focus_for_the_grace_period_stops_it() {
        let clock = Clock::default();
        let stop = AtomicBool::new(false);
        // The watch alone moves the clock: the screen never waits.
        let presence = || false;
        let gone = watch(presence, &stop, || clock.now(), || clock.advance(ms(100)));
        assert!(gone && stop.load(Ordering::Relaxed));
        assert!(clock.now() >= GRACE && clock.now() < GRACE + ms(200));
    }

    #[test]
    fn keeping_focus_keeps_the_watch_going_until_stopped() {
        let clock = Clock::default();
        let stop = AtomicBool::new(false);
        let gone = watch(
            || true,
            &stop,
            || clock.now(),
            || {
                clock.advance(ms(100));
                if clock.now() > ms(5000) {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(!gone);
    }
}
