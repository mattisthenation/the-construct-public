//! Tiny, cheap "digital rain" (Matrix motif) for the dashboard.
//!
//! CPU-safety: this is driven entirely by the dashboard's existing render tick
//! (~5fps via the 200ms event poll). `step` advances the animation exactly once
//! per frame and does only O(columns) integer work — no threads, no busy loop,
//! no `SystemTime` per cell. Randomness comes from a seeded xorshift PRNG (no
//! `rand` dependency), so the panel is deterministic and trivially testable.

/// Characters the rain falls through. Fixed set → no allocation per cell.
const GLYPHS: &[u8] = b"01<>[]{}#$%&*+=01ACEHKLNRTZ";

/// A single falling column: a head position and a trail length.
#[derive(Clone, Copy)]
struct Drop {
    /// Row of the leading (brightest) glyph. Wraps past the bottom.
    head: u16,
    /// How many rows of trail follow the head.
    len: u16,
    /// Per-column speed (rows advanced per N frames); 1 = fastest.
    speed: u16,
}

/// Animation state for the rain panel. Sized to the panel on first draw and
/// re-sized cheaply if the terminal changes.
pub struct Rain {
    width: u16,
    height: u16,
    drops: Vec<Drop>,
    frame: u64,
    rng: u32,
}

impl Default for Rain {
    fn default() -> Self {
        Self::new()
    }
}

/// One xorshift32 step. Deterministic, fast, no dependencies.
fn xorshift(mut x: u32) -> u32 {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

impl Rain {
    pub fn new() -> Self {
        Rain {
            width: 0,
            height: 0,
            drops: Vec::new(),
            frame: 0,
            // Non-zero seed (xorshift requires it); folded with frame on each step.
            rng: 0x1234_5678,
        }
    }

    fn next_rand(&mut self) -> u32 {
        self.rng = xorshift(self.rng ^ (self.frame as u32).wrapping_mul(0x9E37_79B1));
        self.rng
    }

    /// Resize the column set to match the panel. Bounded: columns == width.
    fn resize(&mut self, width: u16, height: u16) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.drops = (0..width as usize)
            .map(|i| {
                let r = xorshift(self.rng ^ (i as u32).wrapping_mul(2_654_435_761));
                Drop {
                    head: (r % height.max(1) as u32) as u16,
                    len: 2 + (r >> 8) as u16 % height.max(2),
                    speed: 1 + (r >> 16) as u16 % 3,
                }
            })
            .collect();
    }

    /// Advance the animation one frame. Call once per render.
    pub fn step(&mut self, width: u16, height: u16) {
        self.resize(width, height);
        if self.height == 0 {
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        for d in &mut self.drops {
            // Advance only on frames divisible by the column's speed → varied rates.
            if self.frame.is_multiple_of(d.speed as u64) {
                d.head = (d.head + 1) % self.height;
            }
        }
        // Occasionally re-roll a column's trail length so it doesn't look static.
        if self.width > 0 {
            let idx = (self.next_rand() % self.width as u32) as usize;
            let r = self.next_rand();
            let h = self.height;
            self.drops[idx].len = 2 + (r % h.max(2) as u32) as u16;
        }
    }

    /// Glyph and brightness tier for a cell, if the rain occupies it.
    /// Returns `(char, tier)` where tier 0 = head (brightest), higher = dimmer.
    /// Pure given current state → testable.
    pub fn cell(&self, col: u16, row: u16) -> Option<(char, u8)> {
        if col >= self.width || row >= self.height {
            return None;
        }
        let d = self.drops.get(col as usize)?;
        // Distance below the head, wrapping the panel height.
        let dist = (self.height + d.head - row) % self.height;
        if dist > d.len {
            return None;
        }
        // Deterministic glyph per (col,row,head) so it shimmers but isn't noisy.
        let seed = xorshift(
            (col as u32).wrapping_mul(73_856_093)
                ^ (row as u32).wrapping_mul(19_349_663)
                ^ (d.head as u32).wrapping_mul(83_492_791),
        );
        let g = GLYPHS[(seed as usize) % GLYPHS.len()] as char;
        let tier = if dist == 0 {
            0
        } else if dist * 3 < d.len {
            1
        } else {
            2
        };
        Some((g, tier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_is_bounded_and_advances() {
        let mut r = Rain::new();
        r.step(10, 5);
        assert_eq!(r.drops.len(), 10);
        let before = r.frame;
        r.step(10, 5);
        assert_eq!(r.frame, before + 1);
        // Heads stay within the panel.
        for d in &r.drops {
            assert!(d.head < 5);
        }
    }

    #[test]
    fn zero_size_is_safe() {
        let mut r = Rain::new();
        r.step(0, 0);
        assert!(r.cell(0, 0).is_none());
    }

    #[test]
    fn head_cell_is_brightest_tier() {
        let mut r = Rain::new();
        r.step(4, 6);
        // The head row of column 0 must render at tier 0.
        let head = r.drops[0].head;
        assert_eq!(r.cell(0, head).map(|(_, t)| t), Some(0));
    }

    #[test]
    fn out_of_bounds_returns_none() {
        let mut r = Rain::new();
        r.step(4, 4);
        assert!(r.cell(99, 0).is_none());
        assert!(r.cell(0, 99).is_none());
    }
}
