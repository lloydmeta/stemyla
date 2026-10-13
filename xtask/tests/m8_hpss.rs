//! M8 acceptance: HPSS parts add back up at margin 1, and the percussive
//! output is much closer to the true drums than the raw mix is.

use engine::hpss::HpssConfig;
use xtask::separate::classic_on_synth;

#[test]
fn classic_drums_beat_the_raw_mix() {
    let r = classic_on_synth(&HpssConfig::default(), "default");
    println!(
        "percussive vs drums {:.2} dB, raw mix vs drums {:.2} dB, harmonic vs non-drums {:.2} dB (mix {:.2} dB), reconstruction error {:.1e}",
        r.sdr_percussive, r.sdr_mix, r.sdr_harmonic, r.sdr_mix_vs_harmonic_truth, r.recon_err
    );
    assert!(
        r.recon_err < 1e-4,
        "H + P differs from the mix by {}",
        r.recon_err
    );
    assert!(
        r.sdr_percussive > r.sdr_mix + 6.0,
        "percussive {:.2} dB is not clearly above the raw mix {:.2} dB",
        r.sdr_percussive,
        r.sdr_mix
    );
}
