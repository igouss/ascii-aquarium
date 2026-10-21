//! The small schooling fish both scenes share.

/// How one kind of bait fish schools. Different mixes produce different
/// collective shapes (Couzin et al. 2002): strong alignment gives polarised
/// schools and ribbons, low alignment gives swarms, and a tangential pull
/// gives a mill.
#[derive(Debug, Clone, Copy)]
pub struct Species {
    pub cruise: f32,
    pub max_speed: f32,
    pub min_speed: f32,
    /// Relative odds of a new school being this species.
    pub weight: f32,
    pub school_size: (i32, i32),
    /// Personal space in grid cells (x, y), so every fish gets its own glyph.
    pub spacing: (f32, f32),
    pub align: f32,
    pub cohesion: f32,
    pub jitter: f32,
    /// Tangential pull around the local centre: >0 makes a rotating bait ball.
    pub mill: f32,
    /// Damps vertical speed; real fish mostly cruise level.
    pub vertical_drag: f32,
}

pub const SPECIES: [Species; 5] = [
    // sardine
    Species {
        cruise: 36.0,
        max_speed: 64.0,
        min_speed: 18.0,
        weight: 3.0,
        school_size: (18, 28),
        spacing: (1.8, 1.1),
        align: 1.6,
        cohesion: 1.6,
        jitter: 40.0,
        mill: 70.0,
        vertical_drag: 0.5,
    },
    // anchovy
    Species {
        cruise: 46.0,
        max_speed: 78.0,
        min_speed: 22.0,
        weight: 3.0,
        school_size: (20, 32),
        spacing: (1.5, 1.0),
        align: 3.2,
        cohesion: 0.45,
        jitter: 30.0,
        mill: 0.0,
        vertical_drag: 1.4,
    },
    // silverside
    Species {
        cruise: 40.0,
        max_speed: 70.0,
        min_speed: 20.0,
        weight: 3.0,
        school_size: (12, 20),
        spacing: (2.0, 1.1),
        align: 2.4,
        cohesion: 1.1,
        jitter: 35.0,
        mill: 0.0,
        vertical_drag: 1.6,
    },
    // mackerel
    Species {
        cruise: 32.0,
        max_speed: 60.0,
        min_speed: 16.0,
        weight: 2.0,
        school_size: (5, 9),
        spacing: (2.6, 1.3),
        align: 2.0,
        cohesion: 0.9,
        jitter: 25.0,
        mill: 0.0,
        vertical_drag: 2.0,
    },
    // goby
    Species {
        cruise: 26.0,
        max_speed: 56.0,
        min_speed: 10.0,
        weight: 1.0,
        school_size: (8, 12),
        spacing: (1.4, 1.1),
        align: 0.35,
        cohesion: 1.4,
        jitter: 110.0,
        mill: 0.0,
        vertical_drag: 0.3,
    },
];

/// A species index, weighted by [`Species::weight`].
pub fn pick_species(rng: &mut fastrand::Rng) -> usize {
    let total: f32 = SPECIES.iter().map(|s| s.weight).sum();
    let mut r: f32 = rng.f32() * total;
    for (i, species) in SPECIES.iter().enumerate() {
        r -= species.weight;
        if r <= 0.0 {
            return i;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_follow_the_weights() {
        let mut rng = fastrand::Rng::with_seed(7);
        let mut counts = [0u32; SPECIES.len()];
        for _ in 0..12_000 {
            counts[pick_species(&mut rng)] += 1;
        }
        // Weights 3:3:3:2:1 of 12.
        for (count, expected) in counts.iter().zip([3000, 3000, 3000, 2000, 1000]) {
            assert!(count.abs_diff(expected) < 250, "{counts:?}");
        }
    }
}
