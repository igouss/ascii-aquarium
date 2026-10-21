//! A frame of terminal cells, and the grid that maps pixels onto them.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]

use super::math::Rgb;

/// A quantised colour, as a terminal takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    #[must_use]
    pub fn from_rgb(c: Rgb) -> Self {
        let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        Self(q(c[0]), q(c[1]), q(c[2]))
    }

    #[must_use]
    pub fn rgb(self) -> Rgb {
        [f32::from(self.0), f32::from(self.1), f32::from(self.2)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub glyph: char,
    pub fg: Color,
    pub bg: Color,
}

/// The cell grid in pixels: `cols` by `rows` cells of `cw` by `ch` pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cw: f32,
    pub ch: f32,
}

impl Grid {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.cols * self.rows
    }

    #[must_use]
    pub fn width(&self) -> f32 {
        self.cols as f32 * self.cw
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.rows as f32 * self.ch
    }

    /// The cell at pixel (x, y), if it is on the grid.
    #[must_use]
    pub fn index(&self, x: f32, y: f32) -> Option<usize> {
        let c = (x / self.cw).floor();
        let r = (y / self.ch).floor();
        (c >= 0.0 && r >= 0.0 && c < self.cols as f32 && r < self.rows as f32)
            .then(|| r as usize * self.cols + c as usize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub cols: usize,
    pub rows: usize,
    /// What the frame was cleared to: the water, under everything drawn.
    pub bg: Color,
    pub cells: Vec<Cell>,
}

impl Frame {
    #[must_use]
    pub fn new(cols: usize, rows: usize, bg: Color) -> Self {
        let blank = Cell {
            glyph: ' ',
            fg: bg,
            bg,
        };
        Self {
            cols,
            rows,
            bg,
            cells: vec![blank; cols * rows],
        }
    }

    pub fn clear(&mut self, bg: Color) {
        self.bg = bg;
        self.cells.fill(Cell {
            glyph: ' ',
            fg: bg,
            bg,
        });
    }

    /// Draws `glyph` in `color` at `alpha` over whatever background cell `i` has.
    pub fn glyph(&mut self, i: usize, glyph: char, color: Rgb, alpha: f32) {
        let cell = &mut self.cells[i];
        let a = alpha.clamp(0.0, 1.0);
        cell.fg = Color::from_rgb(super::math::mix(cell.bg.rgb(), color, a));
        cell.glyph = glyph;
    }

    /// Fills cell `i` with a solid block of `color`.
    pub fn block(&mut self, i: usize, color: Rgb) {
        self.cells[i].bg = Color::from_rgb(color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_round_and_clamp() {
        assert_eq!(Color::from_rgb([-3.0, 127.5, 300.0]), Color(0, 128, 255));
    }

    #[test]
    fn grid_indexes_cells_and_rejects_the_outside() {
        let grid = Grid {
            cols: 4,
            rows: 3,
            cw: 10.0,
            ch: 20.0,
        };
        assert_eq!(grid.index(0.0, 0.0), Some(0));
        assert_eq!(grid.index(39.9, 59.9), Some(11));
        assert_eq!(grid.index(15.0, 25.0), Some(5));
        for (x, y) in [(-0.1, 0.0), (0.0, -0.1), (40.0, 0.0), (0.0, 60.0)] {
            assert_eq!(grid.index(x, y), None, "({x}, {y})");
        }
    }

    #[test]
    fn glyphs_blend_over_the_cell_background() {
        let mut frame = Frame::new(2, 1, Color(0, 0, 100));
        frame.block(1, [200.0, 0.0, 0.0]);
        frame.glyph(0, '-', [255.0, 255.0, 255.0], 0.5);
        frame.glyph(1, '#', [0.0, 0.0, 0.0], 1.0);
        assert_eq!(
            frame.cells[0],
            Cell {
                glyph: '-',
                fg: Color(128, 128, 178),
                bg: Color(0, 0, 100)
            }
        );
        assert_eq!(
            frame.cells[1],
            Cell {
                glyph: '#',
                fg: Color(0, 0, 0),
                bg: Color(200, 0, 0)
            }
        );
    }
}
