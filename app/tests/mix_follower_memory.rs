//! MEM-1 and MEM-4: a change of the mix, with spectrograms on, holds no
//! song-sized buffer at all (D6 of the M7 plan).  The mix lane's waveform
//! job and its spectrogram read the lanes in blocks.  One test in its own
//! binary, since it counts every allocation of the process.
#![cfg(not(target_arch = "wasm32"))]

use app::export::{BLOCK, MixFollower};
use app::lanes::{LaneAudio, LanesMix};
use app::spectrogram::SpectrogramJob;
use engine::mixdown::{MixRecipe, Pcm};
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

/// A block of both channels, in bytes.
const BLOCK_BYTES: usize = BLOCK * 2 * size_of::<f32>();

#[test]
fn a_mix_change_with_spectrograms_on_holds_no_song_sized_buffer() {
    // 256 blocks: one channel of the song is 16 MiB.
    let frames = 256 * BLOCK;
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
    let lanes: Vec<LaneAudio> = song.stems.iter().map(LaneAudio::of_stem).collect();
    let mix_of = |recipe: &MixRecipe| -> Box<dyn Pcm> {
        Box::new(LanesMix::new(&lanes, frames, 44_100, recipe))
    };
    let mut follower = MixFollower::new(MixRecipe::new(vec![1.0, 1.0]));
    let no_drums = MixRecipe::new(vec![1.0, 0.0]);
    let no_bass = MixRecipe::new(vec![0.0, 1.0]);

    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    assert!(follower.follow(&no_drums, mix_of, || false).is_none());
    let waveform = follower.follow(&no_bass, mix_of, || true);
    assert!(waveform.is_some(), "the new mix is shown");
    // With spectrograms on, the mix lane's spectrogram is made again.
    let mut spectrogram = SpectrogramJob::new(mix_of(follower.shown().expect("a mix")));
    assert!(spectrogram.run_while(|| true));
    let image = spectrogram.finish();
    let grew = PEAK.load(Relaxed) - before;
    println!("a mix change with its spectrogram peaks {grew} bytes above its start");
    assert!(image.1 > 1_000, "the image has its columns");
    let bound = 16 * BLOCK_BYTES;
    assert!(
        grew < bound,
        "MEM-1: a mix change with spectrograms on held {grew} more bytes at its peak; the bound is {bound} bytes, and one channel of the song is {} bytes",
        frames * size_of::<f32>()
    );
}
