//! MEM-4: a change of the mix while the mix lane's waveform job runs never
//! holds two song-sized mid buffers at once.  One test in its own binary,
//! since it counts every allocation of the process.
#![cfg(not(target_arch = "wasm32"))]

use app::export::{BLOCK, MixFollower};
use engine::mixdown::MixRecipe;
use engine::{Song, Stem};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(live, Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

#[test]
fn a_new_mix_frees_the_old_jobs_mid_before_it_makes_its_own() {
    let frames = 64 * BLOCK;
    let stem = |name: &str| Stem {
        name: name.into(),
        left: vec![0.25; frames].into(),
        right: vec![-0.25; frames].into(),
    };
    let song = Arc::new(Song {
        sample_rate: 44_100,
        frames,
        stems: vec![stem("bass"), stem("drums")],
    });
    let mut follower = MixFollower::new(MixRecipe::new(vec![1.0, 1.0]));
    let no_drums = MixRecipe::new(vec![1.0, 0.0]);
    let no_bass = MixRecipe::new(vec![0.0, 1.0]);
    assert!(follower.follow(&song, &no_drums, true, || false).is_none());

    let mid = frames * size_of::<f32>();
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    assert!(follower.follow(&song, &no_bass, true, || false).is_none());
    let grew = PEAK.load(Relaxed) - before;
    assert!(
        grew < mid / 2,
        "MEM-4: a mix change held {grew} more bytes at its peak; one mid buffer is {mid}"
    );
}
