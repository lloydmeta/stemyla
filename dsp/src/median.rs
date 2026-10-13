/// Finds the middle value of a list after sorting it.
///
/// The median ignores a few very loud or very quiet values, which makes it
/// good at removing clicks and drum hits from a steady sound.  It only works
/// on lists with an odd number of values, so there is always one middle.
///
/// The original list is not changed: sorting happens on a copy, using bubble
/// sort.  Bubble sort walks along the list, swapping neighbours that are in
/// the wrong order, and repeats until nothing needs swapping.
pub fn median(values: &[f32]) -> f32 {
    assert!(
        values.len() % 2 == 1,
        "median needs an odd number of values, got {}",
        values.len()
    );

    let mut copy = Vec::new();
    let mut i = 0;
    while i < values.len() {
        copy.push(values[i]);
        i = i + 1;
    }

    let mut swapped = true;
    while swapped {
        swapped = false;
        let mut j = 0;
        while j + 1 < copy.len() {
            if copy[j] > copy[j + 1] {
                let temp = copy[j];
                copy[j] = copy[j + 1];
                copy[j + 1] = temp;
                swapped = true;
            }
            j = j + 1;
        }
    }

    copy[copy.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_of_five() {
        assert_eq!(median(&[5.0, 3.0, 2.0, 8.0, 2.0]), 3.0);
    }

    #[test]
    fn ignores_one_big_spike() {
        assert_eq!(median(&[0.1, 0.1, 9.0, 0.1, 0.1]), 0.1);
    }

    #[test]
    fn does_not_change_the_input() {
        let values = vec![3.0, 1.0, 2.0];
        median(&values);
        assert_eq!(values, vec![3.0, 1.0, 2.0]);
    }

    #[test]
    #[should_panic(expected = "odd number")]
    fn even_length_panics() {
        median(&[1.0, 2.0]);
    }

    #[test]
    #[should_panic(expected = "odd number")]
    fn empty_panics() {
        median(&[]);
    }
}
