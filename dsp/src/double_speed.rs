/// Makes a sound play twice as fast by keeping every second sample.
///
/// The sound gets shorter and higher, like a chipmunk.
pub fn double_speed(samples: &[f32]) -> Vec<f32> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        result.push(samples[i]);
        i = i + 2;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_every_second_sample() {
        assert_eq!(
            double_speed(&[1.0, 2.0, 3.0, 4.0, 5.0]),
            vec![1.0, 3.0, 5.0]
        );
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(double_speed(&[]), vec![]);
    }
}
