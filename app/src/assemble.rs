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
    /// A segment of this separation could not be placed.  The worker still
    /// sends the rest of it; those segments and its `done` or `error` are
    /// dropped, so the failure is reported once and cannot end a later
    /// separation the user started meanwhile.
    failed: bool,
}

impl Assembly {
    /// One "segment" message, the first of a separation if `first` (it
    /// starts at sample 0): `place` writes its stems (with [`Self::place`]
    /// and [`Self::advance`]).  On an error the partial stems are dropped
    /// and the rest of this separation is ignored; the error is returned
    /// once.
    pub fn segment(
        &mut self,
        first: bool,
        place: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        if first {
            self.failed = false;
        }
        if self.failed {
            return Ok(());
        }
        place(self).inspect_err(|_| {
            self.clear();
            self.failed = true;
        })
    }

    /// The worker finished the separation: its stems, or `None` if it had
    /// already failed here.
    pub fn done(&mut self) -> Option<Result<Vec<Stem>, String>> {
        if self.failed {
            self.clear();
            return None;
        }
        // A short separation's partial stems would otherwise stay until the
        // next one starts: six song-length stems, hundreds of MB.
        Some(self.take().inspect_err(|_| self.clear()))
    }

    /// The worker's separation failed: drops the partial stems.  False if
    /// it had already failed here, so it is not reported twice.
    pub fn abandon(&mut self) -> bool {
        let first = !self.failed;
        self.clear();
        first
    }

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

    /// Marks samples up to `end` as filled once all `count` stems have
    /// their segment, which must be every stem.
    pub fn advance(&mut self, count: usize, end: usize) -> Result<(), String> {
        if count != self.stems.len() {
            return Err(format!(
                "a segment with {count} stems, not {}",
                self.stems.len()
            ));
        }
        self.filled = end;
        Ok(())
    }

    /// The finished stems, if every sample has arrived.
    pub fn take(&mut self) -> Result<Vec<Stem>, String> {
        let Some(len) = self.stems.first().map(|s| s.1.len()) else {
            return Err("the separation ended before any samples arrived".into());
        };
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
        self.failed = false;
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
        a.advance(seg.len(), start + n)
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

    fn feed(
        a: &mut Assembly,
        start: usize,
        len: usize,
        seg: &[(&str, Vec<f32>)],
    ) -> Result<(), String> {
        a.segment(start == 0, |a| put(a, start, len, seg))
    }

    #[test]
    fn a_separation_with_nothing_placed_is_an_error() {
        assert!(Assembly::default().take().is_err());
        assert!(Assembly::default().done().unwrap().is_err());
    }

    #[test]
    fn segments_fed_in_order_are_done_once_complete() {
        let mut a = Assembly::default();
        feed(&mut a, 0, 3, &[("bass", vec![1.0, 2.0])]).unwrap();
        feed(&mut a, 2, 3, &[("bass", vec![3.0])]).unwrap();
        assert_eq!(a.done().unwrap().unwrap()[0].1, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn after_a_failed_segment_the_rest_of_that_separation_is_dropped() {
        let mut a = Assembly::default();
        feed(&mut a, 0, 6, &[("bass", vec![1.0, 2.0])]).unwrap();
        assert!(feed(&mut a, 3, 6, &[("bass", vec![1.0])]).is_err(), "a gap");
        assert!(a.stems.is_empty(), "the partial stems are freed");
        assert_eq!(feed(&mut a, 2, 6, &[("bass", vec![1.0])]), Ok(()));
        assert!(a.stems.is_empty());
        assert!(a.done().is_none(), "the failure was already reported");
    }

    #[test]
    fn a_worker_error_after_a_failed_segment_is_not_reported_again() {
        let mut a = Assembly::default();
        assert!(feed(&mut a, 4, 6, &[("bass", vec![1.0])]).is_err());
        assert!(!a.abandon());
        assert!(
            a.done().unwrap().is_err(),
            "a later separation is reported again"
        );
    }

    #[test]
    fn a_new_separation_after_a_failed_one_is_assembled() {
        let mut a = Assembly::default();
        assert!(feed(&mut a, 4, 6, &[("bass", vec![1.0])]).is_err());
        feed(&mut a, 0, 2, &[("bass", vec![7.0, 8.0])]).unwrap();
        assert_eq!(a.done().unwrap().unwrap()[0].1, vec![7.0, 8.0]);
    }

    #[test]
    fn a_worker_error_drops_the_partial_stems_and_is_reported() {
        let mut a = Assembly::default();
        feed(&mut a, 0, 6, &[("bass", vec![1.0, 2.0])]).unwrap();
        assert!(a.abandon());
        assert!(a.stems.is_empty());
        assert!(a.done().unwrap().is_err());
    }

    #[test]
    fn a_separation_that_ends_short_frees_its_partial_stems() {
        let mut a = Assembly::default();
        feed(&mut a, 0, 1000, &[("bass", vec![1.0; 400])]).unwrap();
        assert!(a.done().unwrap().is_err(), "400 of 1000 samples");
        assert!(a.stems.is_empty());
    }

    #[test]
    fn a_segment_missing_a_stem_is_an_error() {
        let mut a = Assembly::default();
        let both = [("bass", vec![1.0]), ("drums", vec![2.0])];
        feed(&mut a, 0, 3, &both).unwrap();
        assert!(feed(&mut a, 1, 3, &both[..1]).is_err());
        assert!(a.stems.is_empty(), "the failed separation is dropped");
    }
}
