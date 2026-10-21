//! What input means to the screensaver: a deliberate key, click or mouse
//! movement dismisses it, but not the tail of whatever woke the session (a
//! key still held, a pointer still settling), nor jitter. The dismissal
//! policy is commit-screensaver's, which copied the shader screensaver's.

use std::{fmt, time::Duration};

/// Input from the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// A key press, or pasted text.
    Key,
    /// A click or scroll.
    Pointer,
    /// The pointer moved into this cell.
    PointerMotion { column: u16, row: u16 },
}

/// Why the screensaver stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    KeyPress,
    ClickOrScroll,
    MouseMoved,
    FocusLeft,
    /// A signal: the terminal closed, or another monitor's instance stopped.
    WindowClosed,
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::KeyPress => "key press",
            Self::ClickOrScroll => "click or scroll",
            Self::MouseMoved => "mouse moved",
            Self::FocusLeft => "focus left the screensaver or the session locked",
            Self::WindowClosed => "window closed",
        })
    }
}

/// Keys and clicks are ignored this soon after the screensaver starts.
const ARMED_AFTER: Duration = Duration::from_millis(500);
/// Pointer motion is ignored for longer, while the pointer settles.
const MOTION_ARMED_AFTER: Duration = Duration::from_millis(1500);
/// Pointer travel, in cells, that is still jitter rather than moving the mouse.
const JITTER_CELLS: u32 = 1;

/// Decides which input dismisses the screensaver.
#[derive(Debug, Clone, Default)]
pub struct Dismissal {
    /// The cell the pointer was last seen in.
    pointer: Option<(u16, u16)>,
    /// Cells travelled since motion was armed.
    travel: u32,
}

impl Dismissal {
    /// Whether `input`, `elapsed` after the screensaver started, dismisses it.
    pub fn judge(&mut self, input: Input, elapsed: Duration) -> Option<StopReason> {
        let armed = elapsed > ARMED_AFTER;
        match input {
            Input::Key => armed.then_some(StopReason::KeyPress),
            Input::Pointer => armed.then_some(StopReason::ClickOrScroll),
            Input::PointerMotion { column, row } => {
                let from = self.pointer.replace((column, row));
                if elapsed <= MOTION_ARMED_AFTER {
                    return None;
                }
                if let Some((from_column, from_row)) = from {
                    self.travel +=
                        u32::from(from_column.abs_diff(column)) + u32::from(from_row.abs_diff(row));
                }
                (self.travel > JITTER_CELLS).then_some(StopReason::MouseMoved)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StopReason::{ClickOrScroll, KeyPress, MouseMoved};
    use proptest::collection::vec;
    use proptest::prelude::*;

    const EARLY: Duration = Duration::from_millis(400);
    const ARMED: Duration = Duration::from_millis(600);
    const SETTLED: Duration = Duration::from_millis(1600);

    const fn moved(column: u16, row: u16) -> Input {
        Input::PointerMotion { column, row }
    }

    #[test]
    fn keys_and_clicks_dismiss_once_armed() {
        let mut dismissal = Dismissal::default();
        for elapsed in [Duration::ZERO, EARLY, ARMED_AFTER] {
            assert_eq!(dismissal.judge(Input::Key, elapsed), None);
            assert_eq!(dismissal.judge(Input::Pointer, elapsed), None);
        }
        assert_eq!(dismissal.judge(Input::Key, ARMED), Some(KeyPress));
        assert_eq!(dismissal.judge(Input::Pointer, ARMED), Some(ClickOrScroll));
    }

    #[test]
    fn motion_waits_for_the_pointer_to_settle() {
        let mut dismissal = Dismissal::default();
        assert_eq!(dismissal.judge(moved(0, 0), ARMED), None);
        assert_eq!(dismissal.judge(moved(40, 20), MOTION_ARMED_AFTER), None);
        // Travel before then doesn't count, but where the pointer ended up does.
        assert_eq!(dismissal.judge(moved(41, 20), SETTLED), None);
        assert_eq!(dismissal.judge(moved(42, 20), SETTLED), Some(MouseMoved));
    }

    #[test]
    fn travel_of_two_cells_is_a_move() {
        let mut dismissal = Dismissal::default();
        assert_eq!(
            dismissal.judge(moved(5, 5), SETTLED),
            None,
            "nowhere to come from"
        );
        assert_eq!(dismissal.judge(moved(6, 6), SETTLED), Some(MouseMoved));
    }

    #[test]
    fn a_cell_of_jitter_is_not_a_move() {
        let mut dismissal = Dismissal::default();
        for cell in [moved(5, 5), moved(5, 6), moved(5, 6)] {
            assert_eq!(dismissal.judge(cell, SETTLED), None);
        }
    }

    #[test]
    fn reasons_read_like_the_shader_screensavers() {
        assert_eq!(KeyPress.to_string(), "key press");
        assert_eq!(ClickOrScroll.to_string(), "click or scroll");
        assert_eq!(MouseMoved.to_string(), "mouse moved");
        assert_eq!(
            StopReason::FocusLeft.to_string(),
            "focus left the screensaver or the session locked"
        );
        assert_eq!(StopReason::WindowClosed.to_string(), "window closed");
    }

    fn input() -> impl Strategy<Value = Input> {
        prop_oneof![
            Just(Input::Key),
            Just(Input::Pointer),
            (0u16..300, 0u16..100).prop_map(|(column, row)| moved(column, row)),
        ]
    }

    proptest! {
        #[test]
        fn nothing_dismisses_before_the_arm_time(inputs in vec(input(), 0..40), ms in 0u64..=500) {
            let mut dismissal = Dismissal::default();
            for input in inputs {
                prop_assert_eq!(dismissal.judge(input, Duration::from_millis(ms)), None);
            }
        }

        #[test]
        fn motion_dismisses_once_travel_passes_a_cell(cells in vec((0u16..300, 0u16..100), 1..30)) {
            let mut dismissal = Dismissal::default();
            let mut travel = 0;
            for (i, &(column, row)) in cells.iter().enumerate() {
                if let Some(&(from_column, from_row)) = i.checked_sub(1).and_then(|j| cells.get(j)) {
                    travel += u32::from(from_column.abs_diff(column)) + u32::from(from_row.abs_diff(row));
                }
                let verdict = dismissal.judge(moved(column, row), SETTLED);
                prop_assert_eq!(verdict, (travel > 1).then_some(MouseMoved));
                if verdict.is_some() {
                    break;
                }
            }
        }
    }
}
