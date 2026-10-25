/// Mixes one piece of several stems.
///
/// It does what `mix` does, but only for the `length` samples from `start`.
/// So a long song can be mixed one small piece at a time, and the whole mix
/// never has to be in memory at once.
///
/// There must be one volume for every stem, and every stem must reach the end
/// of the piece.  With no stems at all, the piece is silence.
pub fn mix_range(stems: &[&[f32]], volumes: &[f32], start: usize, length: usize) -> Vec<f32> {
    assert!(
        volumes.len() == stems.len(),
        "every stem needs a volume: got {} stems and {} volumes",
        stems.len(),
        volumes.len()
    );

    let end = start + length;
    let mut s = 0;
    while s < stems.len() {
        assert!(
            stems[s].len() >= end,
            "every stem must reach the end of the piece: stem {} has {} samples, the piece ends at {}",
            s,
            stems[s].len(),
            end
        );
        s = s + 1;
    }

    let mut result = vec![0.0; length];
    let mut i = 0;
    while i < length {
        let mut total = 0.0;
        let mut s = 0;
        while s < stems.len() {
            total = total + stems[s][start + i] * volumes[s];
            s = s + 1;
        }

        if total > 1.0 {
            total = 1.0;
        }
        if total < -1.0 {
            total = -1.0;
        }
        result[i] = total;
        i = i + 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mix;

    #[test]
    fn mix_range_matches_mix_at_any_start() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut i = 0;
        while i < 50 {
            a.push((i % 7) as f32 * 0.2 - 0.6);
            b.push((i % 5) as f32 * 0.25 - 0.5);
            i = i + 1;
        }
        let whole = mix(&[&a, &b], &[0.5, 1.25]);

        let mut start = 0;
        while start <= 50 {
            let mut length = 0;
            while start + length <= 50 {
                let piece = mix_range(&[&a, &b], &[0.5, 1.25], start, length);
                assert_eq!(piece.len(), length);
                let mut k = 0;
                while k < length {
                    assert_eq!(piece[k], whole[start + k], "start {start}, sample {k}");
                    k = k + 1;
                }
                length = length + 1;
            }
            start = start + 1;
        }
    }

    #[test]
    fn clamps_to_one() {
        let a = vec![0.9, -0.9, 0.5];
        let b = vec![0.9, -0.9, 0.25];
        assert_eq!(
            mix_range(&[&a, &b], &[1.0, 1.0], 0, 3),
            vec![1.0, -1.0, 0.75]
        );
    }

    #[test]
    fn no_stems_is_silence() {
        assert_eq!(mix_range(&[], &[], 10, 3), vec![0.0, 0.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "every stem needs a volume")]
    fn missing_volume_panics() {
        mix_range(&[&[0.5], &[0.25]], &[1.0], 0, 1);
    }

    #[test]
    #[should_panic(expected = "reach the end of the piece")]
    fn a_short_stem_panics() {
        mix_range(&[&[0.5, 0.1, 0.2], &[0.25, 0.3]], &[1.0, 1.0], 1, 2);
    }
}
