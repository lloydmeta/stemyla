/// Mixes several stems into one sound.
///
/// Every stem is multiplied by its own volume, then all the stems are added
/// together sample by sample.  If the total goes above 1.0 or below -1.0 it is
/// clamped, because speakers can't go further than that.
///
/// There must be at least one stem, one volume for every stem, and every stem
/// must be the same length.
pub fn mix(stems: &[&[f32]], volumes: &[f32]) -> Vec<f32> {
    assert!(stems.len() > 0, "mix needs at least one stem");
    assert!(
        volumes.len() == stems.len(),
        "every stem needs a volume: got {} stems and {} volumes",
        stems.len(),
        volumes.len()
    );

    let length = stems[0].len();
    let mut s = 1;
    while s < stems.len() {
        assert!(
            stems[s].len() == length,
            "all stems must be the same length: stem 0 has {} samples, stem {} has {}",
            length,
            s,
            stems[s].len()
        );
        s = s + 1;
    }

    let mut result = Vec::new();
    let mut i = 0;
    while i < length {
        let mut total = 0.0;
        let mut s = 0;
        while s < stems.len() {
            total = total + stems[s][i] * volumes[s];
            s = s + 1;
        }

        if total > 1.0 {
            total = 1.0;
        }
        if total < -1.0 {
            total = -1.0;
        }
        result.push(total);
        i = i + 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_stem_at_full_volume_is_unchanged() {
        let stem = vec![0.1, -0.2, 0.3];
        assert_eq!(mix(&[&stem], &[1.0]), stem);
    }

    #[test]
    fn all_volumes_at_zero_is_silence() {
        let a = vec![0.5, 0.25];
        let b = vec![0.25, 0.5];
        assert_eq!(mix(&[&a, &b], &[0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn adds_stems_together() {
        let a = vec![0.5, 0.25];
        let b = vec![0.25, 0.5];
        assert_eq!(mix(&[&a, &b], &[1.0, 1.0]), vec![0.75, 0.75]);
    }

    #[test]
    fn clamps_to_one() {
        let a = vec![0.9, -0.9];
        let b = vec![0.9, -0.9];
        assert_eq!(mix(&[&a, &b], &[1.0, 1.0]), vec![1.0, -1.0]);
    }

    #[test]
    #[should_panic(expected = "every stem needs a volume")]
    fn missing_volume_panics() {
        let a = vec![0.5];
        let b = vec![0.25];
        mix(&[&a, &b], &[1.0]);
    }

    #[test]
    #[should_panic(expected = "same length")]
    fn different_lengths_panic() {
        mix(&[&[0.5, 0.1], &[0.25]], &[1.0, 1.0]);
    }

    #[test]
    #[should_panic(expected = "at least one stem")]
    fn no_stems_panics() {
        mix(&[], &[]);
    }
}
