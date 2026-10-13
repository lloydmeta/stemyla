/// Splits the samples into blocks and finds the lowest and highest sample in
/// each block.
///
/// This is how a waveform picture is drawn: one vertical line per block, from
/// the lowest value to the highest.  The last block can be shorter than the
/// others if the samples don't divide evenly.
pub fn peaks(samples: &[f32], block_size: usize) -> Vec<(f32, f32)> {
    assert!(block_size > 0, "block_size must be bigger than 0");

    let mut result = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let mut low = samples[start];
        let mut high = samples[start];

        let mut i = start;
        while i < start + block_size && i < samples.len() {
            if samples[i] < low {
                low = samples[i];
            }
            if samples[i] > high {
                high = samples[i];
            }
            i = i + 1;
        }

        result.push((low, high));
        start = start + block_size;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_block() {
        assert_eq!(peaks(&[0.1, -0.5, 0.3, 0.2], 4), vec![(-0.5, 0.3)]);
    }

    #[test]
    fn two_blocks() {
        assert_eq!(
            peaks(&[0.1, -0.5, 0.9, 0.2], 2),
            vec![(-0.5, 0.1), (0.2, 0.9)]
        );
    }

    #[test]
    fn short_last_block() {
        assert_eq!(peaks(&[0.1, -0.5, 0.9], 2), vec![(-0.5, 0.1), (0.9, 0.9)]);
    }

    #[test]
    fn no_samples_means_no_blocks() {
        assert_eq!(peaks(&[], 3), vec![]);
    }

    #[test]
    #[should_panic(expected = "bigger than 0")]
    fn zero_block_size_panics() {
        peaks(&[0.1], 0);
    }
}
