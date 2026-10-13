/// Finds the largest number in a list of samples.
///
/// "Largest" means furthest to the right on a number line, so -0.2 is
/// bigger than -0.9.  The list must have at least one sample.
pub fn biggest(samples: &[f32]) -> f32 {
    assert!(samples.len() > 0, "biggest needs at least one sample");

    let mut best = samples[0];
    let mut i = 1;
    while i < samples.len() {
        if samples[i] > best {
            best = samples[i];
        }
        i = i + 1;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_biggest() {
        assert_eq!(biggest(&[0.1, 0.7, -0.9, 0.3]), 0.7);
    }

    #[test]
    fn works_with_only_negative_numbers() {
        assert_eq!(biggest(&[-0.5, -0.2, -0.9]), -0.2);
    }

    #[test]
    #[should_panic(expected = "at least one sample")]
    fn empty_list_panics() {
        biggest(&[]);
    }
}
