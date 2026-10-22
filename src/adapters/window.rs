//! The scene in a fullscreen GPUI window: each frame's blocks drawn as one
//! quad per run of a colour, and each non-blank cell as a glyph shaped once
//! and kept; keys, clicks and pointer motion fed to the session.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "cell counts and pointer cells of a window, far below 2^23 and floored at 0"
)]

use std::{
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, Font, FontId, FontWeight, GlyphId, Hsla, KeyDownEvent, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Render, Rgba, ScrollWheelEvent, TextRun, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px,
    size,
};

use screensaver_session::{dismissal::Input, stop::StopReason};

use crate::{
    app::Session,
    domain::frame::{Color, Frame, Grid},
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

/// A glyph as shaped on its own: the face that has it, its id there, and
/// where it sits from the cell's baseline origin.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    font: FontId,
    id: GlyphId,
    offset: Point<Pixels>,
}

/// Every glyph the scene has drawn, shaped once at the grid's font size.
/// Every frame's text differs, so shaping whole rows each frame missed
/// GPUI's line cache every time and cost several times the simulation.
#[derive(Debug, Clone)]
struct Glyphs {
    grid: Grid,
    size: Pixels,
    /// From a cell's top to the baseline, as a shaped line centres it.
    baseline: Pixels,
    by_char: HashMap<char, Option<Glyph>>,
}

impl Glyphs {
    fn new(grid: Grid, window: &Window) -> Self {
        let size = px((grid.ch * GLYPH_SCALE).round());
        let line = shape(window, 'M', size);
        let padding = (px(grid.ch) - line.ascent - line.descent) / 2.0;
        Self {
            grid,
            size,
            baseline: padding + line.ascent,
            by_char: HashMap::new(),
        }
    }

    /// Shapes the glyphs in `frame` not seen before.
    fn learn(&mut self, frame: &Frame, window: &Window) {
        for cell in frame.cells.iter().filter(|c| c.glyph != ' ') {
            self.by_char.entry(cell.glyph).or_insert_with(|| {
                let line = shape(window, cell.glyph, self.size);
                let run = line.runs.first()?;
                let glyph = run.glyphs.first()?;
                Some(Glyph {
                    font: run.font_id,
                    id: glyph.id,
                    offset: glyph.position,
                })
            });
        }
    }
}

fn shape(window: &Window, glyph: char, size: Pixels) -> Arc<gpui::LineLayout> {
    let run = TextRun {
        len: glyph.len_utf8(),
        font: Font {
            weight: FontWeight::SEMIBOLD,
            ..gpui::font(FONT)
        },
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .layout_line(glyph.encode_utf8(&mut [0; 4]), size, &[run], None)
}

struct Aquarium {
    session: Session,
    started: Instant,
    grid: Grid,
    glyphs: Option<Rc<Glyphs>>,
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
        let glyphs = match &mut self.glyphs {
            Some(glyphs) if glyphs.grid == grid => glyphs,
            stale => stale.insert(Rc::new(Glyphs::new(grid, window))),
        };
        Rc::make_mut(glyphs).learn(&frame, window);
        let glyphs = Rc::clone(glyphs);
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
                    move |bounds, (), window, _| paint(&frame, &glyphs, bounds, window),
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

/// Paints `frame` over a base of its background: a quad per run of another
/// background in a row, then the glyph of every non-blank cell.
fn paint(frame: &Frame, glyphs: &Glyphs, bounds: Bounds<Pixels>, window: &mut Window) {
    window.paint_quad(gpui::fill(bounds, hsla(frame.bg)));
    let Grid { cw, ch, .. } = glyphs.grid;
    for (r, row) in frame.cells.chunks(frame.cols.max(1)).enumerate() {
        let top = bounds.origin + point(px(0.0), px(r as f32 * ch));
        let mut col = 0;
        for run in row.chunk_by(|a, b| a.bg == b.bg) {
            if run[0].bg != frame.bg {
                let at = top + point(px(col as f32 * cw), px(0.0));
                let extent = size(px(run.len() as f32 * cw), px(ch));
                window.paint_quad(gpui::fill(Bounds::new(at, extent), hsla(run[0].bg)));
            }
            col += run.len();
        }
        for (c, cell) in row.iter().enumerate() {
            let Some(Some(glyph)) = glyphs.by_char.get(&cell.glyph) else {
                continue;
            };
            let origin = top + point(px(c as f32 * cw), glyphs.baseline) + glyph.offset;
            let _ = window.paint_glyph(origin, glyph.font, glyph.id, glyphs.size, hsla(cell.fg));
        }
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
                    glyphs: None,
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
