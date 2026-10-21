//! The scene in a fullscreen GPUI window: each frame's cells drawn as one
//! shaped line per row, a colour run per change, with cell backgrounds from
//! the runs; keys, clicks and pointer motion fed to the session.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "cell counts and pointer cells of a window, far below 2^23 and floored at 0"
)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, Font, FontWeight, Hsla, KeyDownEvent, MouseDownEvent, MouseMoveEvent,
    Pixels, Render, Rgba, ScrollWheelEvent, SharedString, TextAlign, TextRun, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px,
    size,
};

use crate::{
    app::Session,
    domain::{
        frame::{Color, Frame, Grid},
        input::{Input, StopReason},
    },
};

/// The originals draw their glyphs at this fraction of the row height.
const GLYPH_SCALE: f32 = 0.74;
const FONT: &str = "JetBrainsMono Nerd Font";

/// How the window ends: the session's own stops, plus a signal or the focus
/// watch from outside.
pub struct Exits {
    /// Set by a signal: the window was closed.
    pub stop: Arc<AtomicBool>,
    /// Set by the focus watch.
    pub focus_left: Arc<AtomicBool>,
    /// Called once with why the session stopped, before the app quits.
    pub on_stop: Box<dyn FnOnce(StopReason)>,
}

struct Aquarium {
    session: Session,
    started: Instant,
    grid: Grid,
    exits: Option<Exits>,
    focus: gpui::FocusHandle,
}

impl Aquarium {
    fn input(&mut self, input: Input, cx: &Context<Self>) {
        if let Some(reason) = self.session.input(input, self.started.elapsed()) {
            self.stop(reason, cx);
        }
    }

    fn stop(&mut self, reason: StopReason, cx: &Context<Self>) {
        if let Some(exits) = self.exits.take() {
            (exits.on_stop)(reason);
            cx.quit();
        }
    }

    /// Whether something outside has stopped the session.
    fn poll_exits(&mut self, cx: &Context<Self>) {
        let outside = self.exits.as_ref().and_then(|e| {
            if e.focus_left.load(Ordering::Relaxed) {
                Some(StopReason::FocusLeft)
            } else {
                e.stop
                    .load(Ordering::Relaxed)
                    .then_some(StopReason::WindowClosed)
            }
        });
        if let Some(reason) = outside {
            self.stop(reason, cx);
        }
    }
}

impl Render for Aquarium {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        self.grid = self
            .session
            .grid(f32::from(viewport.width), f32::from(viewport.height));
        let grid = self.grid;
        let frame = self.session.advance(grid, self.started.elapsed()).clone();
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| this.input(Input::Key, cx)))
            .on_any_mouse_down(
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.input(Input::Pointer, cx)),
            )
            .on_scroll_wheel(
                cx.listener(|this, _: &ScrollWheelEvent, _, cx| this.input(Input::Pointer, cx)),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                let column = (f32::from(e.position.x) / this.grid.cw).max(0.0) as u16;
                let row = (f32::from(e.position.y) / this.grid.ch).max(0.0) as u16;
                this.input(Input::PointerMotion { column, row }, cx);
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| paint(&frame, grid, bounds, window, cx),
                )
                .size_full(),
            )
    }
}

fn hsla(c: Color) -> Hsla {
    Rgba {
        r: f32::from(c.0) / 255.0,
        g: f32::from(c.1) / 255.0,
        b: f32::from(c.2) / 255.0,
        a: 1.0,
    }
    .into()
}

/// Paints `frame` row by row: one shaped line each, with a run per change
/// of colour, every glyph forced to the cell width.
fn paint(frame: &Frame, grid: Grid, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(first) = frame.cells.first() else {
        return;
    };
    window.paint_quad(gpui::fill(bounds, hsla(first.bg)));
    let font = Font {
        weight: FontWeight::SEMIBOLD,
        ..gpui::font(FONT)
    };
    let font_size = px((grid.ch * GLYPH_SCALE).round());
    let line_height = px(grid.ch);
    let mut text = String::with_capacity(frame.cols * 2);
    let mut runs: Vec<TextRun> = Vec::new();
    for (r, row) in frame.cells.chunks(frame.cols.max(1)).enumerate() {
        text.clear();
        runs.clear();
        for cell in row {
            let start = text.len();
            text.push(cell.glyph);
            let len = text.len() - start;
            let (color, background) = (hsla(cell.fg), (cell.bg != first.bg).then(|| hsla(cell.bg)));
            match runs.last_mut() {
                Some(run) if run.color == color && run.background_color == background => {
                    run.len += len;
                }
                _ => runs.push(TextRun {
                    len,
                    font: font.clone(),
                    color,
                    background_color: background,
                    underline: None,
                    strikethrough: None,
                }),
            }
        }
        let line = window.text_system().shape_line(
            SharedString::from(text.clone()),
            font_size,
            &runs,
            Some(px(grid.cw)),
        );
        let origin = point(bounds.origin.x, bounds.origin.y + px(r as f32 * grid.ch));
        let _ = line.paint_background(origin, line_height, TextAlign::Left, None, window, cx);
        let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
    }
}

/// Opens the fullscreen window with Wayland app id `app_id` and runs
/// `session` in it at `fps` until it stops; returns once the app quits.
pub fn run(session: Session, fps: u32, app_id: String, exits: Exits) {
    let frame_time = Duration::from_secs(1) / fps.max(1);
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(720.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Fullscreen(bounds)),
            titlebar: None,
            focus: true,
            show: true,
            app_id: Some(app_id),
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                // The frame clock: redraw at the frame rate, and look out
                // for stops from outside between frames.
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(frame_time).await;
                        let alive = this.update(cx, |this: &mut Aquarium, cx| {
                            this.poll_exits(cx);
                            cx.notify();
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                })
                .detach();
                Aquarium {
                    session,
                    started: Instant::now(),
                    grid: Grid {
                        cols: 0,
                        rows: 0,
                        cw: 1.0,
                        ch: 1.0,
                    },
                    exits: Some(exits),
                    focus,
                }
            })
        });
        if opened.is_err() {
            cx.quit();
        }
        cx.activate(true);
    });
}
