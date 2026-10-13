/// Keeps a play position inside a loop.
///
/// When the position reaches `loop_end`, it jumps back to `loop_start`.  If it
/// went a little past the end, it keeps that little bit (the overshoot), so
/// no samples are skipped or repeated.
pub fn wrap_loop(position: usize, loop_start: usize, loop_end: usize) -> usize {
    assert!(
        loop_start < loop_end,
        "loop_start ({}) must be before loop_end ({})",
        loop_start,
        loop_end
    );

    let mut p = position;
    let length = loop_end - loop_start;
    while p >= loop_end {
        p = p - length;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_the_end_stays_put() {
        assert_eq!(wrap_loop(499, 100, 500), 499);
    }

    #[test]
    fn exactly_at_the_end_jumps_to_start() {
        assert_eq!(wrap_loop(500, 100, 500), 100);
    }

    #[test]
    fn overshoot_is_kept() {
        assert_eq!(wrap_loop(503, 100, 500), 103);
    }

    #[test]
    fn far_past_the_end_wraps_several_times() {
        assert_eq!(wrap_loop(903, 100, 500), 103);
    }

    #[test]
    #[should_panic(expected = "must be before")]
    fn backwards_loop_panics() {
        wrap_loop(10, 500, 100);
    }
}
