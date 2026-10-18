//! Song-length stems put together from the segments the separation worker
//! sends after each chunk, so neither thread ever holds two copies of all
//! six stems.

/// One stem: (name, left, right).
pub type Stem = (String, Vec<f32>, Vec<f32>);

/// Stems being filled in, in the worker's order.
#[derive(Default)]
pub struct Assembly {
    pub stems: Vec<Stem>,
    /// Samples filled so far, counted from the start of the song.
    pub filled: usize,
}

impl Assembly {
    /// Writes one stem's samples `start..start + left.len()` of a song of
    /// `len` samples.  The first segment of a song sets the stems up;
    /// segments must arrive in order and without gaps.
    pub fn place(
        &mut self,
        index: usize,
        name: &str,
        start: usize,
        len: usize,
        write: impl FnOnce(&mut [f32], &mut [f32]),
        n: usize,
    ) -> Result<(), String> {
        if start == 0 && index == 0 {
            self.stems.clear();
            self.filled = 0;
        }
        if index == self.stems.len() {
            if start != 0 {
                return Err(format!("stem {name} starts at sample {start}, not 0"));
            }
            self.stems
                .push((name.to_string(), vec![0.0; len], vec![0.0; len]));
        }
        let (have, l, r) = self
            .stems
            .get_mut(index)
            .ok_or_else(|| format!("stem {index} ({name}) arrived before stem {}", index - 1))?;
        if have != name || l.len() != len {
            return Err(format!(
                "segment for {name} ({len} samples) does not match {have} ({} samples)",
                l.len()
            ));
        }
        if start != self.filled || start + n > len {
            return Err(format!(
                "segment {start}..{} of {name} does not follow sample {} of {len}",
                start + n,
                self.filled
            ));
        }
        write(&mut l[start..start + n], &mut r[start..start + n]);
        Ok(())
    }

    /// Marks samples up to `end` as filled once every stem has its segment.
    pub fn advance(&mut self, end: usize) {
        self.filled = end;
    }

    /// The finished stems, if every sample has arrived.
    pub fn take(&mut self) -> Result<Vec<Stem>, String> {
        let len = self.stems.first().map_or(0, |s| s.1.len());
        if self.filled != len {
            return Err(format!(
                "the separation ended after {} of {len} samples",
                self.filled
            ));
        }
        self.filled = 0;
        Ok(std::mem::take(&mut self.stems))
    }

    pub fn clear(&mut self) {
        self.stems.clear();
        self.filled = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(
        a: &mut Assembly,
        start: usize,
        len: usize,
        seg: &[(&str, Vec<f32>)],
    ) -> Result<(), String> {
        let n = seg[0].1.len();
        for (i, (name, v)) in seg.iter().enumerate() {
            a.place(
                i,
                name,
                start,
                len,
                |l, r| {
                    l.copy_from_slice(v);
                    for (d, s) in r.iter_mut().zip(v) {
                        *d = -s;
                    }
                },
                n,
            )?;
        }
        a.advance(start + n);
        Ok(())
    }

    #[test]
    fn segments_fill_song_length_stems_in_order() {
        let mut a = Assembly::default();
        put(
            &mut a,
            0,
            5,
            &[("bass", vec![1.0, 2.0]), ("drums", vec![5.0, 6.0])],
        )
        .unwrap();
        put(
            &mut a,
            2,
            5,
            &[
                ("bass", vec![3.0, 4.0, 4.5]),
                ("drums", vec![7.0, 8.0, 8.5]),
            ],
        )
        .unwrap();
        let stems = a.take().unwrap();
        assert_eq!(stems.len(), 2);
        assert_eq!(stems[0].0, "bass");
        assert_eq!(stems[0].1, vec![1.0, 2.0, 3.0, 4.0, 4.5]);
        assert_eq!(stems[0].2, vec![-1.0, -2.0, -3.0, -4.0, -4.5]);
        assert_eq!(stems[1].1, vec![5.0, 6.0, 7.0, 8.0, 8.5]);
        assert!(a.stems.is_empty());
    }

    #[test]
    fn a_gap_or_a_short_song_is_an_error() {
        let mut a = Assembly::default();
        put(&mut a, 0, 6, &[("bass", vec![1.0, 2.0])]).unwrap();
        assert!(put(&mut a, 3, 6, &[("bass", vec![1.0])]).is_err());
        assert!(a.take().is_err(), "only 2 of 6 samples arrived");
    }

    #[test]
    fn a_new_song_starts_over() {
        let mut a = Assembly::default();
        put(&mut a, 0, 4, &[("bass", vec![1.0, 2.0])]).unwrap();
        put(&mut a, 0, 2, &[("bass", vec![9.0, 9.0])]).unwrap();
        assert_eq!(a.take().unwrap()[0].1, vec![9.0, 9.0]);
    }
}
