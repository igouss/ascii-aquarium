//! The Screen port for real: the terminal on stdin and stdout, in raw mode on
//! the alternate screen, reporting every mouse move. Output never blocks for
//! long: a terminal that stops reading, or is gone, ends the show as a closed
//! window instead of hanging it.

#![expect(
    clippy::cast_precision_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]

use std::{
    io,
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    io::Errno,
    stdio::{stdin, stdout},
    termios::{OptionalActions, Termios, tcgetattr, tcgetwinsize, tcsetattr},
};

use crate::{
    domain::{
        ansi,
        frame::Frame,
        input::{Decoder, Input},
    },
    ports::Screen,
};

/// Alternate screen, hidden cursor, no autowrap, any-motion mouse reports in
/// SGR form, and bold glyphs (the originals use a 600-weight font).
const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?1003h\x1b[?1006h\x1b[1m";
const LEAVE: &[u8] = b"\x1b[0m\x1b[?1006l\x1b[?1003l\x1b[?7h\x1b[?25h\x1b[?1049l";
/// Cell size to assume when the terminal doesn't report pixels.
const FALLBACK_CELL: (f32, f32) = (11.0, 22.0);
/// A terminal that takes no output for this long has stopped reading.
const STALL: Duration = Duration::from_secs(2);

/// The terminal is gone: the app reads this as the window closing.
fn gone(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, format!("the terminal {why}"))
}

/// Writes all of `bytes` to the non-blocking stdout, waiting while the
/// terminal catches up, but no longer than [`STALL`] at a time.
fn write_all(mut bytes: &[u8]) -> io::Result<()> {
    let mut stalled_since: Option<Instant> = None;
    while !bytes.is_empty() {
        match rustix::io::write(stdout(), bytes) {
            Ok(n) => {
                bytes = &bytes[n..];
                stalled_since = None;
            }
            Err(Errno::AGAIN) => {
                let since = *stalled_since.get_or_insert_with(Instant::now);
                let left = STALL.saturating_sub(since.elapsed());
                if left.is_zero() {
                    return Err(gone("stopped reading"));
                }
                let out = stdout();
                let mut fds = [PollFd::new(&out, PollFlags::OUT)];
                let timeout = Timespec::try_from(left).unwrap_or(Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                });
                match poll(&mut fds, Some(&timeout)) {
                    Ok(_) if fds[0].revents().intersects(PollFlags::HUP | PollFlags::ERR) => {
                        return Err(gone("went away"));
                    }
                    Ok(_) | Err(Errno::INTR) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Err(Errno::INTR) => {}
            Err(Errno::IO | Errno::PIPE) => return Err(gone("went away")),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Terminal {
    saved: Termios,
    saved_flags: OFlags,
    shown: Option<Frame>,
    out: Vec<u8>,
    decoder: Decoder,
}

impl Terminal {
    /// Takes over the terminal until dropped.
    ///
    /// # Errors
    ///
    /// If stdin isn't a terminal.
    pub fn open() -> io::Result<Self> {
        let saved = tcgetattr(stdin())?;
        let mut raw = saved.clone();
        raw.make_raw();
        tcsetattr(stdin(), OptionalActions::Flush, &raw)?;
        let saved_flags = fcntl_getfl(stdout())?;
        fcntl_setfl(stdout(), saved_flags | OFlags::NONBLOCK)?;
        write_all(ENTER)?;
        Ok(Self {
            saved,
            saved_flags,
            shown: None,
            out: Vec::new(),
            decoder: Decoder::default(),
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = write_all(LEAVE);
        let _ = fcntl_setfl(stdout(), self.saved_flags);
        let _ = tcsetattr(stdin(), OptionalActions::Flush, &self.saved);
    }
}

impl Screen for Terminal {
    fn size(&mut self) -> io::Result<(usize, usize, f32, f32)> {
        let size = tcgetwinsize(stdout())?;
        let (cols, rows) = (
            usize::from(size.ws_col.max(1)),
            usize::from(size.ws_row.max(1)),
        );
        let (cw, ch) = if size.ws_xpixel > 0 && size.ws_ypixel > 0 {
            (
                f32::from(size.ws_xpixel) / cols as f32,
                f32::from(size.ws_ypixel) / rows as f32,
            )
        } else {
            FALLBACK_CELL
        };
        Ok((cols, rows, cw, ch))
    }

    fn show(&mut self, frame: &Frame) -> io::Result<()> {
        self.out.clear();
        ansi::encode(self.shown.as_ref(), frame, &mut self.out);
        write_all(&self.out)?;
        match &mut self.shown {
            Some(shown) if shown.cells.len() == frame.cells.len() => shown.clone_from(frame),
            shown => *shown = Some(frame.clone()),
        }
        Ok(())
    }

    fn wait(&mut self, timeout: Duration, input: &mut dyn FnMut(Input)) -> io::Result<()> {
        let stdin_fd = stdin();
        let mut fds = [PollFd::new(&stdin_fd, PollFlags::IN)];
        let timeout = Timespec::try_from(timeout).unwrap_or(Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        });
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => {
                self.decoder.flush(&mut |i| input(i));
                return Ok(());
            }
            Ok(_) => {}
            Err(Errno::INTR) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        if fds[0].revents().intersects(PollFlags::HUP | PollFlags::ERR) {
            return Err(gone("went away"));
        }
        let mut buf = [0u8; 4096];
        // Straight from the descriptor: std's buffered stdin could hold bytes
        // poll can't see.
        // stdin shares stdout's open file, so it is non-blocking too.
        let n = match rustix::io::read(stdin(), &mut buf) {
            Ok(0) | Err(Errno::IO) => return Err(gone("went away")),
            Ok(n) => n,
            Err(Errno::AGAIN | Errno::INTR) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        self.decoder.feed(&buf[..n], &mut |i| input(i));
        Ok(())
    }
}
