//! Turning a predator's spine into cells: a smooth, undulating render spine,
//! body coordinates for any point near it, and supersampled coverage per cell.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]

use super::frame::Grid;
use super::math::clamp;
use super::pond::{NODES, nonzero};

/// Sub-cell sample offsets for anti-aliased silhouettes.
const SUBSAMPLES: [(f32, f32); 4] = [(0.25, 0.3), (0.75, 0.3), (0.25, 0.72), (0.75, 0.72)];

const SEGMENTS: usize = NODES - 1;

/// The render spine of one predator this frame.
#[derive(Debug, Clone)]
pub struct Pose {
    spine: [f32; NODES * 2],
    arc: [f32; NODES],
    arc_total: f32,
    segments: Segments,
}

/// Each spine segment's start, run and squared length, worked out once per
/// pose rather than once per sample, and laid out so the nearest-segment
/// search is a straight loop the compiler vectorises: it is most of the
/// cost of drawing a frame.
#[derive(Debug, Clone)]
struct Segments {
    ax: [f32; SEGMENTS],
    ay: [f32; SEGMENTS],
    sx: [f32; SEGMENTS],
    sy: [f32; SEGMENTS],
    len2: [f32; SEGMENTS],
}

impl Segments {
    fn new(spine: &[f32; NODES * 2]) -> Self {
        let mut segments = Self {
            ax: [0.0; SEGMENTS],
            ay: [0.0; SEGMENTS],
            sx: [0.0; SEGMENTS],
            sy: [0.0; SEGMENTS],
            len2: [0.0; SEGMENTS],
        };
        for k in 0..SEGMENTS {
            let (ax, ay) = (spine[k * 2], spine[k * 2 + 1]);
            let sx = spine[(k + 1) * 2] - ax;
            let sy = spine[(k + 1) * 2 + 1] - ay;
            segments.ax[k] = ax;
            segments.ay[k] = ay;
            segments.sx[k] = sx;
            segments.sy[k] = sy;
            segments.len2[k] = nonzero(sx.mul_add(sx, sy * sy), 1.0);
        }
        segments
    }
}

/// Where a point sits relative to the spine.
#[derive(Debug, Clone, Copy)]
pub struct Nearest {
    /// Distance from the spine.
    pub dist: f32,
    /// Position along the spine, 0 snout to 1 tail.
    pub s: f32,
    /// Which side of the spine: 1 or -1.
    pub sign: f32,
    /// Unit tangent of the nearest segment.
    pub tangent: (f32, f32),
    /// How far past the tail tip the point lies, along the last segment.
    pub past_tail: f32,
}

impl Pose {
    /// Bends the sim spine `nodes` sideways by `offset(s)` at each node, for
    /// the travelling body wave.
    #[must_use]
    pub fn new(nodes: &[f32; NODES * 2], offset: impl Fn(f32) -> f32) -> Self {
        let mut spine = [0.0; NODES * 2];
        for k in 0..NODES {
            let s = k as f32 / (NODES - 1) as f32;
            let a = k.saturating_sub(1);
            let b = (k + 1).min(NODES - 1);
            let tx = nodes[a * 2] - nodes[b * 2];
            let ty = nodes[a * 2 + 1] - nodes[b * 2 + 1];
            let tl = nonzero(tx.hypot(ty), 1.0);
            let off = offset(s);
            spine[k * 2] = (ty / tl).mul_add(-off, nodes[k * 2]);
            spine[k * 2 + 1] = (tx / tl).mul_add(off, nodes[k * 2 + 1]);
        }
        let mut arc = [0.0; NODES];
        for k in 1..NODES {
            let seg = (spine[k * 2] - spine[(k - 1) * 2])
                .hypot(spine[k * 2 + 1] - spine[(k - 1) * 2 + 1]);
            arc[k] = arc[k - 1] + seg;
        }
        let arc_total = nonzero(arc[NODES - 1], 1.0);
        Self {
            segments: Segments::new(&spine),
            spine,
            arc,
            arc_total,
        }
    }

    #[must_use]
    pub fn nearest(&self, px: f32, py: f32) -> Nearest {
        let Segments {
            ax,
            ay,
            sx,
            sy,
            len2,
        } = &self.segments;
        let mut raw = [0.0; SEGMENTS];
        let mut d2 = [0.0; SEGMENTS];
        for k in 0..SEGMENTS {
            raw[k] = (px - ax[k]).mul_add(sx[k], (py - ay[k]) * sy[k]) / len2[k];
            let u = clamp(raw[k], 0.0, 1.0);
            d2[k] = (py - sy[k].mul_add(u, ay[k])).mul_add(
                py - sy[k].mul_add(u, ay[k]),
                (px - sx[k].mul_add(u, ax[k])).powi(2),
            );
        }
        // The first segment at the least distance, as a scan that keeps
        // only strict improvements would find it.
        let mut best: Option<(usize, f32)> = None;
        for (k, &d) in d2.iter().enumerate() {
            if d < best.map_or(f32::INFINITY, |b| b.1) {
                best = Some((k, d));
            }
        }
        let Some((k, d2)) = best else {
            return Nearest {
                dist: f32::INFINITY,
                s: 0.0,
                sign: 1.0,
                tangent: (1.0, 0.0),
                past_tail: 0.0,
            };
        };
        let len = len2[k].sqrt();
        let u = clamp(raw[k], 0.0, 1.0);
        Nearest {
            dist: d2.sqrt(),
            s: u.mul_add(len, self.arc[k]) / self.arc_total,
            sign: if sy[k].mul_add(-(px - ax[k]), sx[k] * (py - ay[k])) >= 0.0 {
                1.0
            } else {
                -1.0
            },
            tangent: (sx[k] / len, sy[k] / len),
            past_tail: if k == SEGMENTS - 1 {
                (raw[k] - 1.0).max(0.0) * len
            } else {
                0.0
            },
        }
    }

    /// The cells within `reach` of the spine, clipped to the grid, as
    /// inclusive (c0, c1, r0, r1).
    #[must_use]
    pub fn bounds(&self, grid: Grid, reach: f32) -> Option<(usize, usize, usize, usize)> {
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for k in 0..NODES {
            min_x = min_x.min(self.spine[k * 2]);
            max_x = max_x.max(self.spine[k * 2]);
            min_y = min_y.min(self.spine[k * 2 + 1]);
            max_y = max_y.max(self.spine[k * 2 + 1]);
        }
        let c0 = ((min_x - reach) / grid.cw).floor().max(0.0);
        let c1 = ((max_x + reach) / grid.cw)
            .floor()
            .min((grid.cols - 1) as f32);
        let r0 = ((min_y - reach) / grid.ch).floor().max(0.0);
        let r1 = ((max_y + reach) / grid.ch)
            .floor()
            .min((grid.rows - 1) as f32);
        (c0 <= c1 && r0 <= r1).then_some((c0 as usize, c1 as usize, r0 as usize, r1 as usize))
    }
}

/// One point classified against a body.
#[derive(Debug, Clone, Copy)]
pub struct Sample<H> {
    pub at: Nearest,
    /// What the point hit, if it is on the body.
    pub hit: Option<H>,
}

/// A cell the body covers at least partly.
#[derive(Debug, Clone, Copy)]
pub struct Covered<H> {
    pub index: usize,
    pub col: usize,
    pub row: usize,
    /// The sample at the cell centre.
    pub centre: Sample<H>,
    /// The first hit, centre first, then the sub-samples in order.
    pub first: H,
    /// Fraction of the five samples on the body.
    pub coverage: f32,
}

/// Supersamples every cell within `reach` of the pose with `sample`, and
/// hands each cell the body touches to `paint`.
pub fn rasterize<H: Copy>(
    grid: Grid,
    pose: &Pose,
    reach: f32,
    mut sample: impl FnMut(f32, f32) -> Sample<H>,
    mut paint: impl FnMut(Covered<H>),
) {
    let Some((c0, c1, r0, r1)) = pose.bounds(grid, reach) else {
        return;
    };
    let skip_beyond = reach + grid.cw.hypot(grid.ch);
    for row in r0..=r1 {
        for col in c0..=c1 {
            let x = col as f32 * grid.cw;
            let y = row as f32 * grid.ch;
            let centre = sample(x + grid.cw / 2.0, y + grid.ch / 2.0);
            if centre.hit.is_none() && centre.at.dist > skip_beyond {
                continue;
            }
            let mut first = centre.hit;
            let mut hits = u8::from(centre.hit.is_some());
            for (sx, sy) in SUBSAMPLES {
                if let Some(hit) = sample(sx.mul_add(grid.cw, x), sy.mul_add(grid.ch, y)).hit {
                    first = first.or(Some(hit));
                    hits += 1;
                }
            }
            let Some(first) = first else { continue };
            paint(Covered {
                index: row * grid.cols + col,
                col,
                row,
                centre,
                first,
                coverage: f32::from(hits) / 5.0,
            });
        }
    }
}

/// Body-anchored key so each glyph rides along with the skin.
#[must_use]
pub fn skin_key(at: Nearest, cw: f32, id: i32) -> u32 {
    let v = at.dist * at.sign;
    super::math::hash3_bits(
        (at.s * 52.0).round() as i32,
        (v / (cw * 0.9)).round() as i32,
        id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::swim::straight_spine;

    #[test]
    fn a_straight_pose_measures_points_along_and_across() {
        // Snout at x = 200, tail at x = 10, heading +x.
        let pose = Pose::new(&straight_spine(200.0, 50.0, 0.0, 190.0), |_| 0.0);
        let mid = pose.nearest(105.0, 60.0);
        assert!((mid.s - 0.5).abs() < 1e-4 && (mid.dist - 10.0).abs() < 1e-4);
        assert!(
            (pose.nearest(105.0, 40.0).sign + mid.sign).abs() < f32::EPSILON,
            "opposite sides"
        );
        let behind = pose.nearest(0.0, 50.0);
        assert!((behind.s - 1.0).abs() < 1e-4 && (behind.past_tail - 10.0).abs() < 1e-3);
        assert!(pose.nearest(150.0, 50.0).past_tail.abs() < f32::EPSILON);
    }

    #[test]
    fn coverage_counts_samples_on_the_body() {
        let grid = Grid {
            cols: 30,
            rows: 10,
            cw: 10.0,
            ch: 10.0,
        };
        let pose = Pose::new(&straight_spine(200.0, 50.0, 0.0, 190.0), |_| 0.0);
        let mut cells = Vec::new();
        // A body 6 px either side of the spine at y = 50.
        rasterize(
            grid,
            &pose,
            20.0,
            |px, py| {
                let at = pose.nearest(px, py);
                Sample {
                    at,
                    hit: (at.dist <= 6.0 && at.past_tail == 0.0 && px <= 203.0).then_some(()),
                }
            },
            |cell| cells.push((cell.row, cell.col, cell.coverage)),
        );
        assert!(cells.iter().all(|&(row, _, _)| row == 4 || row == 5));
        let full = cells.iter().filter(|c| c.2 >= 0.6).count();
        assert!(full > 0 && full < cells.len(), "{cells:?}");
    }
}
