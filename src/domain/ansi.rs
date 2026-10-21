//! Frames as truecolor terminal output, sending only the cells that changed.

use std::io::Write as _;

use super::frame::{Color, Frame};

/// Brackets a frame so the terminal shows it whole (synchronized output).
const BEGIN: &[u8] = b"\x1b[?2026h";
const END: &[u8] = b"\x1b[?2026l";

/// Appends to `out` what turns the screen from `prev` (unknown when None, or
/// of another size) into `next`.
pub fn encode(prev: Option<&Frame>, next: &Frame, out: &mut Vec<u8>) {
    let prev = prev.filter(|p| p.cols == next.cols && p.rows == next.rows);
    out.extend_from_slice(BEGIN);
    let mut cursor: Option<usize> = None;
    let mut colors: Option<(Color, Color)> = None;
    for (i, cell) in next.cells.iter().enumerate() {
        if prev.is_some_and(|p| p.cells[i] == *cell) {
            continue;
        }
        let (row, col) = (i / next.cols, i % next.cols);
        if cursor != Some(i) {
            let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
        }
        if colors != Some((cell.fg, cell.bg)) {
            let (Color(fr, fg, fb), Color(br, bg, bb)) = (cell.fg, cell.bg);
            let _ = write!(out, "\x1b[38;2;{fr};{fg};{fb};48;2;{br};{bg};{bb}m");
            colors = Some((cell.fg, cell.bg));
        }
        let mut utf8 = [0; 4];
        out.extend_from_slice(cell.glyph.encode_utf8(&mut utf8).as_bytes());
        // Without autowrap the cursor sticks at the last column.
        cursor = (col + 1 < next.cols).then_some(i + 1);
    }
    out.extend_from_slice(END);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::new(3, 2, Color(0, 0, 0))
    }

    fn encoded(prev: Option<&Frame>, next: &Frame) -> String {
        let mut out = Vec::new();
        encode(prev, next, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_first_frame_is_sent_whole() {
        let mut next = frame();
        next.glyph(4, '·', [255.0, 0.0, 0.0], 1.0);
        let sent = encoded(None, &next);
        assert_eq!(
            sent,
            "\x1b[?2026h\x1b[1;1H\x1b[38;2;0;0;0;48;2;0;0;0m   \x1b[2;1H \x1b[38;2;255;0;0;48;2;0;0;0m·\x1b[38;2;0;0;0;48;2;0;0;0m \x1b[?2026l"
        );
    }

    #[test]
    fn an_unchanged_frame_sends_nothing_but_the_brackets() {
        assert_eq!(encoded(Some(&frame()), &frame()), "\x1b[?2026h\x1b[?2026l");
    }

    #[test]
    fn only_changed_cells_are_sent() {
        let mut next = frame();
        next.glyph(5, '-', [0.0, 0.0, 0.0], 1.0);
        assert_eq!(
            encoded(Some(&frame()), &next),
            "\x1b[?2026h\x1b[2;3H\x1b[38;2;0;0;0;48;2;0;0;0m-\x1b[?2026l"
        );
    }

    #[test]
    fn a_resize_resends_everything() {
        let small = Frame::new(1, 1, Color(0, 0, 0));
        assert_eq!(encoded(Some(&small), &frame()).matches(' ').count(), 6);
    }
}
