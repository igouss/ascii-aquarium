//! Each species' body outline and fin layout, as fractions of its length.

use crate::domain::math::{Rgb, smoothstep};
use crate::domain::shark::SharkKind;

/// Cephalofoil: half-span, how far its tips sweep back, its depth, and the
/// notch in the middle of the front edge, as fractions of length.
#[derive(Debug, Clone, Copy)]
pub(super) struct Head {
    pub(super) span: f32,
    pub(super) sweep: f32,
    pub(super) depth: f32,
    pub(super) notch: f32,
}

/// Body outline and fin layout per species, as fractions of length.
#[derive(Debug, Clone, Copy)]
pub(super) struct Shape {
    pub(super) half_width: fn(f32) -> f32,
    /// Pectoral fin: leading root, trailing root, tip position, and reach.
    pub(super) pectoral: [f32; 4],
    /// Notched (falcate) trailing edge on the pectorals.
    pub(super) falcate: bool,
    /// Pelvic fins: leading root, trailing root, tip, reach.
    pub(super) pelvic: [f32; 4],
    /// Forked caudal fin reach, or 0 when the tail is a ribbon.
    pub(super) caudal: f32,
    pub(super) dorsal: [f32; 2],
    pub(super) second_dorsal: [f32; 2],
    pub(super) head: Option<Head>,
    /// None when the eyes sit on the cephalofoil tips.
    pub(super) eye: Option<[f32; 2]>,
    pub(super) gills: [f32; 2],
    /// Skin colour multipliers.
    pub(super) tint: Rgb,
}

/// Classic top-down half-width at s (snout to tail), as a fraction of length.
fn classic_half_width(s: f32) -> f32 {
    torpedo(
        s,
        (0.13, 0.09, 0.95),
        0.016,
        0.3,
        0.087,
        1.25,
        (0.84, 0.019),
        0.012,
    )
}

/// The usual half-width curve: a rounded snout of `snout` width up to `s1`,
/// widening by `swell` to `s2`, tapering from `girth` at `s2` with `taper`
/// down to a stalk of `stalk` at `s3`, then a tail of width `tail`.
#[expect(
    clippy::too_many_arguments,
    reason = "one curve, eight knobs; named at each call"
)]
fn torpedo(
    s: f32,
    snout: (f32, f32, f32),
    swell: f32,
    s2: f32,
    girth: f32,
    taper: f32,
    stalk: (f32, f32),
    tail: f32,
) -> f32 {
    let (s1, width, round) = snout;
    let (s3, neck) = stalk;
    if s < s1 {
        width * ((s / s1) * std::f32::consts::FRAC_PI_2).sin().powf(round)
    } else if s < s2 {
        swell.mul_add(smoothstep(s1, s2, s), width)
    } else if s < s3 {
        girth.mul_add((1.0 - (s - s2) / (s3 - s2)).powf(taper), neck)
    } else {
        let k = (s - s3) / (1.0 - s3);
        neck.mul_add(1.0 - k, tail * (k * std::f32::consts::PI).sin())
    }
}

#[expect(clippy::too_many_lines, reason = "a table: one arm per species")]
pub(super) const fn shape(kind: SharkKind) -> Shape {
    match kind {
        SharkKind::Classic => CLASSIC,
        // Slim neck under a wide, swept-back cephalofoil, tall first dorsal,
        // long upper tail lobe. Bronze-olive grey.
        SharkKind::Hammerhead => Shape {
            half_width: |s| {
                if s < 0.06 {
                    0.045
                } else if s < 0.3 {
                    0.061f32.mul_add(smoothstep(0.06, 0.3, s), 0.045)
                } else {
                    classic_half_width(s) * 0.92
                }
            },
            head: Some(Head {
                span: 0.17,
                sweep: 0.035,
                depth: 0.09,
                notch: 0.0,
            }),
            pectoral: [0.17, 0.4, 0.5, 0.22],
            pelvic: [0.56, 0.63, 0.67, 0.075],
            caudal: 0.11,
            dorsal: [0.28, 0.44],
            second_dorsal: [0.7, 0.74],
            eye: None,
            gills: [0.16, 0.26],
            tint: [0.96, 0.98, 0.9],
            ..CLASSIC
        },
        // Blunt, square snout on a heavy body; barred back. Warm grey-brown.
        SharkKind::Tiger => Shape {
            half_width: |s| {
                torpedo(
                    s,
                    (0.13, 0.1, 0.45),
                    0.014,
                    0.3,
                    0.095,
                    1.2,
                    (0.84, 0.019),
                    0.012,
                )
            },
            pectoral: [0.15, 0.42, 0.53, 0.25],
            caudal: 0.095,
            dorsal: [0.33, 0.46],
            second_dorsal: [0.71, 0.75],
            eye: Some([0.08, 0.12]),
            gills: [0.19, 0.28],
            tint: [1.02, 0.95, 0.84],
            ..CLASSIC
        },
        // Short rounded head, body done by half-length, then a long ribbon of
        // a tail fin. Broad rounded pectorals, spotted sandy skin.
        SharkKind::Zebra => Shape {
            half_width: |s| {
                if s < 0.55 {
                    torpedo(
                        s,
                        (0.1, 0.075, 0.7),
                        0.025,
                        0.28,
                        0.078,
                        1.1,
                        (0.55, 0.022),
                        0.0,
                    )
                } else {
                    let k = (s - 0.55) / 0.45;
                    0.022f32.mul_add(
                        f32::mul_add(k, -k, 1.0),
                        0.012 * (k * std::f32::consts::PI).sin(),
                    )
                }
            },
            pectoral: [0.12, 0.3, 0.36, 0.19],
            falcate: false,
            pelvic: [0.42, 0.48, 0.51, 0.065],
            caudal: 0.0,
            dorsal: [0.36, 0.44],
            second_dorsal: [0.49, 0.53],
            eye: Some([0.06, 0.09]),
            gills: [0.13, 0.2],
            tint: [1.08, 0.97, 0.74],
            ..CLASSIC
        },
        // Great hammerhead: a broader, nearly straight-edged cephalofoil with
        // a notch at the centre, a towering sickle first dorsal, slate skin.
        SharkKind::GreatHammer => Shape {
            half_width: |s| {
                if s < 0.05 {
                    0.04
                } else if s < 0.3 {
                    0.07f32.mul_add(smoothstep(0.05, 0.3, s), 0.04)
                } else {
                    classic_half_width(s) * 0.95
                }
            },
            head: Some(Head {
                span: 0.21,
                sweep: 0.006,
                depth: 0.085,
                notch: 0.014,
            }),
            pectoral: [0.16, 0.38, 0.49, 0.24],
            pelvic: [0.55, 0.62, 0.67, 0.085],
            caudal: 0.13,
            dorsal: [0.24, 0.45],
            second_dorsal: [0.68, 0.74],
            eye: None,
            gills: [0.15, 0.25],
            tint: [0.86, 0.93, 1.02],
            ..CLASSIC
        },
        // Shortfin mako: a long conical snout, a torpedo body that narrows
        // hard to the tail stalk, a tall crescent tail. Metallic indigo back.
        SharkKind::Mako => Shape {
            half_width: |s| {
                torpedo(
                    s,
                    (0.17, 0.088, 1.5),
                    0.012,
                    0.32,
                    0.086,
                    1.45,
                    (0.86, 0.014),
                    0.01,
                )
            },
            pectoral: [0.18, 0.4, 0.5, 0.21],
            pelvic: [0.58, 0.64, 0.68, 0.06],
            caudal: 0.13,
            dorsal: [0.34, 0.45],
            second_dorsal: [0.76, 0.79],
            eye: Some([0.11, 0.15]),
            gills: [0.2, 0.3],
            tint: [0.78, 0.9, 1.2],
            ..CLASSIC
        },
        // Thresher: a compact body over the front half, then the upper tail
        // lobe as a whip as long as the shark. Big eyes, bronze-violet skin.
        SharkKind::Thresher => Shape {
            half_width: |s| {
                if s < 0.48 {
                    torpedo(
                        s,
                        (0.08, 0.07, 0.75),
                        0.02,
                        0.2,
                        0.078,
                        1.3,
                        (0.48, 0.012),
                        0.0,
                    )
                } else {
                    // The whip: thin, with a slight sickle swell before it tapers out.
                    let k = (s - 0.48) / 0.52;
                    0.012f32.mul_add(1.0 - k, 0.006 * (k * std::f32::consts::PI * 0.9).sin())
                        + 0.002
                }
            },
            pectoral: [0.12, 0.27, 0.36, 0.2],
            pelvic: [0.37, 0.41, 0.44, 0.045],
            caudal: 0.0,
            dorsal: [0.24, 0.32],
            second_dorsal: [0.45, 0.47],
            eye: Some([0.05, 0.1]),
            gills: [0.11, 0.18],
            tint: [1.0, 0.9, 1.08],
            ..CLASSIC
        },
        // Nurse shark: a blunt, broad head, a heavy flattened body with both
        // dorsals set far back, rounded paddle pectorals, a long low tail.
        SharkKind::Nurse => Shape {
            half_width: |s| {
                if s < 0.72 {
                    torpedo(
                        s,
                        (0.08, 0.085, 0.4),
                        0.02,
                        0.3,
                        0.083,
                        1.1,
                        (0.72, 0.022),
                        0.0,
                    )
                } else {
                    let k = (s - 0.72) / 0.28;
                    0.022f32.mul_add(
                        f32::mul_add(k, -k, 1.0),
                        0.014 * (k * std::f32::consts::PI).sin(),
                    )
                }
            },
            pectoral: [0.16, 0.34, 0.4, 0.17],
            falcate: false,
            pelvic: [0.45, 0.52, 0.55, 0.075],
            caudal: 0.0,
            dorsal: [0.5, 0.58],
            second_dorsal: [0.63, 0.69],
            eye: Some([0.05, 0.075]),
            gills: [0.14, 0.22],
            tint: [1.1, 0.96, 0.76],
            ..CLASSIC
        },
    }
}

const CLASSIC: Shape = Shape {
    half_width: classic_half_width,
    pectoral: [0.15, 0.43, 0.55, 0.26],
    falcate: true,
    pelvic: [0.57, 0.65, 0.69, 0.085],
    caudal: 0.085,
    dorsal: [0.33, 0.47],
    second_dorsal: [0.71, 0.76],
    head: None,
    eye: Some([0.1, 0.14]),
    gills: [0.19, 0.29],
    tint: [0.92, 0.97, 1.05],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(clippy::cast_precision_loss, reason = "i <= 100")]
    fn every_shape_has_a_body_and_a_thin_tail() {
        for kind in [
            SharkKind::Classic,
            SharkKind::Hammerhead,
            SharkKind::Zebra,
            SharkKind::Tiger,
            SharkKind::GreatHammer,
            SharkKind::Mako,
            SharkKind::Thresher,
            SharkKind::Nurse,
        ] {
            let w = shape(kind).half_width;
            let widest = (0..=100).map(|i| w(i as f32 / 100.0)).fold(0.0, f32::max);
            assert!(widest > 0.07 && widest < 0.13, "{kind:?}: {widest}");
            assert!(w(0.99) < 0.03, "{kind:?} tail");
            // sin(PI) is a hair below zero in f32.
            assert!((0..=100).all(|i| w(i as f32 / 100.0) > -1e-6), "{kind:?}");
        }
    }
}
