//! What input means to the screensaver: a deliberate key, click or mouse
//! movement dismisses it, but not the tail of whatever woke the session (a
//! key still held, a pointer still settling), nor jitter. The dismissal
//! policy is commit-screensaver's, which copied the shader screensaver's.

use std::{fmt, time::Duration};

/// Input from the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// A key press, or pasted text.
    Key,
    /// A click or scroll.
    Pointer,
    /// The pointer moved into this cell.
    PointerMotion { column: u16, row: u16 },
    /// A mouse button let go.
    Release,
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
            Input::Release => None,
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

const ESC: u8 = 0x1b;

/// Turns terminal input bytes into [`Input`]s. Mouse reports come in SGR
/// form (`ESC [ < b ; column ; row M`, `m` on release); anything else is a key.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    /// The start of an escape sequence the last read cut short.
    pending: Vec<u8>,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], out: &mut impl FnMut(Input)) {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(bytes);
        let mut i = 0;
        while i < buf.len() {
            if buf[i..] == [ESC] {
                // Escape, or the start of a sequence cut short: see `flush`.
                self.pending = vec![ESC];
                return;
            }
            if buf[i] != ESC || buf[i + 1] != b'[' {
                // A lone ESC, an Alt chord or a plain key: one key press per read
                // is all the dismissal needs, so skip ahead.
                out(Input::Key);
                i = next_escape(&buf, i + 1);
                continue;
            }
            // A CSI sequence: parameters, then a final byte in 0x40..=0x7e.
            let Some(end) = buf[i + 2..].iter().position(|b| (0x40..=0x7e).contains(b)) else {
                self.pending = buf[i..].to_vec();
                return;
            };
            let end = i + 2 + end;
            out(csi(&buf[i + 2..end], buf[end]).unwrap_or(Input::Key));
            i = end + 1;
        }
    }
}

impl Decoder {
    /// Called once input goes quiet: a lone ESC left pending was the Escape key.
    pub fn flush(&mut self, out: &mut impl FnMut(Input)) {
        if self.pending == [ESC] {
            out(Input::Key);
        }
        self.pending.clear();
    }
}

fn next_escape(buf: &[u8], from: usize) -> usize {
    buf[from..]
        .iter()
        .position(|&b| b == ESC)
        .map_or(buf.len(), |p| from + p)
}

/// A CSI sequence with `params` and `final_byte` as input: mouse reports are
/// pointer input, a release is nothing, anything else is a key.
fn csi(params: &[u8], final_byte: u8) -> Option<Input> {
    let report = params.strip_prefix(b"<")?;
    let mut fields = std::str::from_utf8(report)
        .ok()?
        .split(';')
        .map(str::parse::<u16>);
    let (Some(Ok(button)), Some(Ok(column)), Some(Ok(row))) =
        (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    match final_byte {
        b'M' if button & 32 != 0 => Some(Input::PointerMotion { column, row }),
        b'M' => Some(Input::Pointer),
        // A release only follows a press, which already counted.
        b'm' => Some(Input::Release),
        _ => None,
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

    fn decoded(chunks: &[&[u8]]) -> Vec<Input> {
        let mut decoder = Decoder::default();
        let mut inputs = Vec::new();
        for chunk in chunks {
            decoder.feed(chunk, &mut |input| inputs.push(input));
        }
        inputs
    }

    #[test]
    fn keys_are_keys() {
        assert_eq!(decoded(&[b"q"]), [Input::Key]);
        let mut decoder = Decoder::default();
        let mut inputs = Vec::new();
        decoder.feed(b"\x1b", &mut |input| inputs.push(input));
        assert_eq!(inputs, [], "maybe the start of a sequence");
        decoder.flush(&mut |input| inputs.push(input));
        assert_eq!(inputs, [Input::Key], "no: Escape");
        assert_eq!(decoded(&[b"\x1b[A"]), [Input::Key], "an arrow");
        assert_eq!(decoded(&[b"ab\x1b[B"]), [Input::Key, Input::Key]);
    }

    #[test]
    fn mouse_reports_are_pointer_input() {
        assert_eq!(decoded(&[b"\x1b[<35;12;7M"]), [moved(12, 7)]);
        assert_eq!(
            decoded(&[b"\x1b[<0;3;4M\x1b[<0;3;4m"]),
            [Input::Pointer, Input::Release]
        );
        assert_eq!(decoded(&[b"\x1b[<64;3;4M"]), [Input::Pointer], "scroll");
        assert_eq!(decoded(&[b"\x1b[<99999;3;4M"]), [Input::Key], "garbled");
    }

    #[test]
    fn a_report_split_across_reads_is_reassembled() {
        assert_eq!(decoded(&[b"\x1b[<35;1", b"2;7M"]), [moved(12, 7)]);
        assert_eq!(decoded(&[b"\x1b", b"[<35;12;7M"]), [moved(12, 7)]);
        assert_eq!(
            decoded(&[b"\x1b[<35;12;7M\x1b[", b"<35;13;7M"]),
            [moved(12, 7), moved(13, 7)]
        );
    }

    #[test]
    fn a_release_never_dismisses() {
        assert_eq!(Dismissal::default().judge(Input::Release, SETTLED), None);
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
