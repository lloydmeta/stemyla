//! The overview of a lane: the waveform that the app draws.  It is
//! [`COLUMNS`] columns at most, each the lowest and the highest sample of
//! both channels in its part of the lane.  The app draws it again at the
//! width of each pixel.
//!
//! [`lane_columns`] makes it from two whole channels.  [`OverviewBuilder`]
//! makes the same columns and scale from blocks of any size, when it knows
//! the frames first, so a decode does not hold the song to make them.

/// The most columns in an overview.
pub const COLUMNS: usize = 4096;

/// The frames in each column of a lane of `frames` frames.  The last
/// column can have fewer.
pub fn column_frames(frames: usize) -> usize {
    frames.div_ceil(COLUMNS).max(1)
}

/// The columns of a lane of `frames` frames: at most [`COLUMNS`].
pub fn column_count(frames: usize) -> usize {
    frames.div_ceil(column_frames(frames))
}

/// Peak columns for a stereo signal: `dsp::peaks` on each channel, then the
/// lower minimum and higher maximum of the two.  The scale makes the
/// loudest column fill the lane.
pub fn lane_columns(left: &[f32], right: &[f32]) -> (Vec<(f32, f32)>, f32) {
    let block = column_frames(left.len());
    let pl = dsp::peaks(left, block);
    let pr = dsp::peaks(right, block);
    let cols: Vec<(f32, f32)> = pl
        .iter()
        .zip(&pr)
        .map(|(a, b)| (a.0.min(b.0), a.1.max(b.1)))
        .collect();
    let scale = overview_scale(&cols);
    (cols, scale)
}

/// The scale that makes the loudest of `cols` fill the lane, with
/// `dsp::biggest`.  A silent lane has the scale 1.  [`lane_columns`] and
/// [`OverviewBuilder`] use it, and so does a reader of an overview file,
/// which holds only the columns.
pub fn overview_scale(cols: &[(f32, f32)]) -> f32 {
    let magnitudes: Vec<f32> = cols.iter().map(|(lo, hi)| lo.abs().max(hi.abs())).collect();
    let loudest = if magnitudes.is_empty() {
        0.0
    } else {
        dsp::biggest(&magnitudes)
    };
    if loudest > 1e-6 { 1.0 / loudest } else { 1.0 }
}

/// Makes the columns of [`lane_columns`] from blocks of a lane.  It holds
/// only the columns, at most [`COLUMNS`].
pub struct OverviewBuilder {
    /// The frames in each column.
    column: usize,
    columns: Vec<(f32, f32)>,
    /// The frames of the open column so far.
    filled: usize,
    /// The lowest and the highest sample of the open column, on each side.
    left: (f32, f32),
    right: (f32, f32),
}

impl OverviewBuilder {
    /// For a lane of `frames` frames.  The columns equal those of
    /// [`lane_columns`] when the blocks hold exactly `frames` frames.
    pub fn new(frames: usize) -> OverviewBuilder {
        OverviewBuilder {
            column: column_frames(frames),
            columns: Vec::with_capacity(column_count(frames)),
            filled: 0,
            left: (0.0, 0.0),
            right: (0.0, 0.0),
        }
    }

    /// Takes the next block of the lane.  `left` and `right` have the same
    /// length.
    pub fn put(&mut self, left: &[f32], right: &[f32]) {
        assert_eq!(left.len(), right.len(), "the sides of a block differ");
        for (&l, &r) in left.iter().zip(right) {
            if self.filled == 0 {
                (self.left, self.right) = ((l, l), (r, r));
            } else {
                // The comparisons of `dsp::peaks`, so a NaN has the same
                // effect.
                self.left = lowest_and_highest(self.left, l);
                self.right = lowest_and_highest(self.right, r);
            }
            self.filled += 1;
            if self.filled == self.column {
                self.close();
            }
        }
    }

    /// The columns and the scale, as [`lane_columns`] gives them.
    pub fn finish(mut self) -> (Vec<(f32, f32)>, f32) {
        if self.filled > 0 {
            self.close();
        }
        let scale = overview_scale(&self.columns);
        (self.columns, scale)
    }

    /// Adds the open column, as `lane_columns` joins the two sides.
    fn close(&mut self) {
        self.columns
            .push((self.left.0.min(self.right.0), self.left.1.max(self.right.1)));
        self.filled = 0;
    }
}

fn lowest_and_highest((low, high): (f32, f32), x: f32) -> (f32, f32) {
    (
        if x < low { x } else { low },
        if x > high { x } else { high },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `frames` samples that move up and down by different steps, from a
    /// small linear congruential generator, so every column has its own
    /// lowest and highest sample on each side.
    fn noise(frames: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..frames)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    }

    fn built(left: &[f32], right: &[f32], block: usize) -> (Vec<(f32, f32)>, f32) {
        let mut builder = OverviewBuilder::new(left.len());
        for (l, r) in left.chunks(block).zip(right.chunks(block)) {
            builder.put(l, r);
        }
        builder.finish()
    }

    fn bits(overview: &(Vec<(f32, f32)>, f32)) -> (Vec<(u32, u32)>, u32) {
        let (cols, scale) = overview;
        let cols = cols
            .iter()
            .map(|(lo, hi)| (lo.to_bits(), hi.to_bits()))
            .collect();
        (cols, scale.to_bits())
    }

    #[test]
    fn the_block_overview_gives_lane_columns() {
        for frames in [0, 1, 4_095, 4_096, 4_097, 1_000_003] {
            let (left, right) = (noise(frames, 1), noise(frames, 2));
            let want = lane_columns(&left, &right);
            assert_eq!(want.0.len(), column_count(frames), "{frames} frames");
            for block in [1, 7, 4_096, 131_072, frames.max(1)] {
                let got = built(&left, &right, block);
                assert!(
                    bits(&got) == bits(&want),
                    "{frames} frames in blocks of {block}"
                );
            }
        }
    }

    /// The scale makes the loudest column fill the lane: 1 / its peak.
    #[test]
    fn the_scale_is_one_over_the_loudest_peak() {
        let left = [0.1, -0.5, 0.2, 0.25];
        let right = [0.0, 0.125, -0.25, 0.0];
        let (_, scale) = lane_columns(&left, &right);
        assert_eq!(scale, 2.0);
        assert_eq!(built(&left, &right, 3).1, 2.0);
    }

    /// A -0.0 after a 0.0 in a column keeps 0.0 as its lowest, as
    /// `dsp::peaks` compares with `<`, and the reverse keeps -0.0.
    #[test]
    fn a_signed_zero_keeps_the_first_of_equal_samples() {
        // 8,192 frames: two in each column.
        for first in [0.0_f32, -0.0] {
            let lane: Vec<f32> = (0..8_192)
                .map(|i| if i % 2 == 0 { first } else { -first })
                .collect();
            assert_eq!(column_frames(lane.len()), 2);
            let want = lane_columns(&lane, &lane);
            assert!(bits(&built(&lane, &lane, 1)) == bits(&want), "{first}");
            assert_eq!(want.0[0].0.to_bits(), first.to_bits(), "{first}");
        }
    }

    #[test]
    fn a_quiet_lane_has_the_scale_of_lane_columns() {
        for level in [0.0, 1e-7, 0.5] {
            let lane = vec![level; 5_000];
            let want = lane_columns(&lane, &lane);
            assert!(bits(&built(&lane, &lane, 333)) == bits(&want), "{level}");
        }
    }

    /// The scale of an overview read from a file (D4 of the M7 plan) is
    /// the scale that `lane_columns` gives with the same columns.
    #[test]
    fn an_overview_scale_is_the_scale_of_lane_columns() {
        for (frames, level) in [(0, 1.0), (1, 0.5), (5_000, 1e-7), (1_000_003, 0.75)] {
            let left: Vec<f32> = noise(frames, 3).iter().map(|x| x * level).collect();
            let right: Vec<f32> = noise(frames, 4).iter().map(|x| x * level).collect();
            let (cols, scale) = lane_columns(&left, &right);
            assert_eq!(
                overview_scale(&cols).to_bits(),
                scale.to_bits(),
                "{frames} frames"
            );
        }
        assert_eq!(overview_scale(&[(-0.25, 0.5)]), 2.0);
        assert_eq!(overview_scale(&[]), 1.0, "no columns");
    }

    #[test]
    fn a_long_lane_has_at_most_4096_columns() {
        for frames in [4_096, 4_097, 1_000_003, 30 * 60 * 44_100] {
            let count = column_count(frames);
            assert!(count <= COLUMNS, "{frames} frames: {count} columns");
            assert!(count * column_frames(frames) >= frames, "{frames}");
        }
    }
}
