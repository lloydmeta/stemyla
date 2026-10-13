/// Fades a sound in from silence.
///
/// Sample number `i` is multiplied by `i / n`, where `n` is how many samples
/// there are.  The first sample becomes silent and each one after gets a bit
/// louder.  Playing a fade-in backwards gives a fade-out.
pub fn fade_in(samples: &[f32]) -> Vec<f32> {
    let n = samples.len();
    let mut result = Vec::new();
    let mut i = 0;
    while i < n {
        let amount = i as f32 / n as f32;
        result.push(samples[i] * amount);
        i = i + 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_ones() {
        assert_eq!(fade_in(&[1.0, 1.0, 1.0, 1.0]), vec![0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(fade_in(&[]), vec![]);
    }
}
