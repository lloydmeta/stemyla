//! Browser tests of the AAC encode of a save (M4 of `plans/active/m4a.md`),
//! with a fake `AudioEncoder`, `AudioDecoder` and `AudioData` in place of
//! the browser's.  CI's Chrome on Linux has no AAC encoder (TD-92), so
//! these tests are the AAC encode's checks in CI.

use super::*;
use crate::web::aac::{AAC_BLOCK, LEAD_IN, OUT_MOST, QUEUE_MOST, set_output_wait_ms};

#[wasm_bindgen(inline_js = r#"
let fake = null;
// Installs the fake codecs on `globalThis`, with the options of the JSON
// text `json`.  Each option has a default:
// * delay: the frames of zeros before the input in the encoder's output.
// * probe_delay: the delay that the fake decoder gives the probe.
// * extra: the packets of zeros that the encoder adds at its flush.
// * description, main_description: the description of the first output of
//   the probe's encoder and of the other encoders, as an array of bytes,
//   or null for none.
// * supported: what isConfigSupported gives.
// * queue, hold_ms: whether `encode` adds to the queue, and the time that
//   the encoder holds each block in it.
// * output_ms: the time that the encoder holds each packet before its
//   output; flush_only: no output before `flush`.
// * error_after: the encoder fails once it holds more frames than this.
// * peak: the value of the probe's impulse in the decode.
// * record: whether the encoder keeps each input sample.
export function fake_aac_install(json) {
  if (fake) throw new Error("a fake AAC is already installed");
  const o = Object.assign({ delay: 2112, probe_delay: null, extra: 3, description: [0x12, 0x10],
    main_description: undefined, supported: true, queue: true, hold_ms: 0, output_ms: 0,
    flush_only: false, error_after: -1, peak: 1.0, record: true }, JSON.parse(json));
  const bytes = (d) => (d === null ? null : new Uint8Array(d));
  const stats = { data_made: 0, data_closed: 0, enc_made: 0, enc_closed: 0, dec_made: 0,
    dec_closed: 0, max_queue: 0, max_lag: 0, max_burst: 0, encoders: [], decoder_description: null };
  const saved = {};
  for (const k of ["AudioEncoder", "AudioDecoder", "AudioData", "EncodedAudioChunk"]) saved[k] = globalThis[k];
  const invalid = (what) => new DOMException(what, "InvalidStateError");

  class FakeAudioData {
    constructor(init) {
      const n = init.numberOfFrames;
      const src = init.data;
      const all = new Float32Array(src.buffer, src.byteOffset, 2 * n);
      this.planes = [all.slice(0, n), all.slice(n, 2 * n)];
      this.numberOfFrames = n;
      this.sampleRate = init.sampleRate;
      this.timestamp = init.timestamp;
      this.closed = false;
      stats.data_made++;
    }
    copyTo(dest, options) { dest.set(this.planes[options.planeIndex].subarray(0, dest.length)); }
    close() { if (!this.closed) { this.closed = true; stats.data_closed++; } }
  }

  class FakeAudioEncoder extends EventTarget {
    static async isConfigSupported(config) { return { supported: o.supported, config }; }
    constructor(init) {
      super();
      this.output = init.output;
      this.fail = init.error;
      this.state = "unconfigured";
      this.encodeQueueSize = 0;
      this.index = stats.encoders.length;
      this.rec = { left: [], right: [], timestamps: [], packets: [], frames_in: 0 };
      stats.encoders.push(this.rec);
      stats.enc_made++;
      this.acc = 0;
      this.fill = 0;
      this.ready = [];
      this.formed = 0;
      this.since = 0;
      this.sent_meta = false;
      this.add_zeros(o.delay);
    }
    configure(config) {
      if (this.state === "closed") throw invalid("closed");
      this.config = config;
      this.state = "configured";
    }
    add_frame(left) {
      this.acc += left;
      this.fill++;
      if (this.fill === 1024) this.form();
    }
    add_zeros(n) { for (let i = 0; i < n; i++) this.add_frame(0); }
    form() {
      const k = this.formed++;
      const b = new Uint8Array(8 + (k % 5));
      const v = new DataView(b.buffer);
      v.setUint32(0, k, true);
      v.setFloat32(4, this.acc, true);
      b.fill(0xab, 8);
      this.ready.push(b);
      this.acc = 0;
      this.fill = 0;
    }
    encode(data) {
      if (this.state !== "configured") throw invalid("not configured");
      if (data.closed) throw new TypeError("a closed AudioData");
      const n = data.numberOfFrames;
      const [left, right] = data.planes;
      const rec = this.rec;
      rec.frames_in += n;
      rec.timestamps.push(data.timestamp);
      if (o.record) { rec.left.push(...left); rec.right.push(...right); }
      if (o.queue) this.encodeQueueSize++;
      stats.max_queue = Math.max(stats.max_queue, this.encodeQueueSize);
      if (this.index > 0) {
        stats.max_lag = Math.max(stats.max_lag, rec.frames_in - 1024 * rec.packets.length);
        stats.max_burst = Math.max(stats.max_burst, this.since);
        this.since = 0;
      }
      if (this.index > 0 && o.error_after >= 0 && rec.frames_in > o.error_after) {
        setTimeout(() => {
          if (this.state === "closed") return;
          this.state = "closed";
          this.encodeQueueSize = 0;
          stats.enc_closed++;
          this.fail(new DOMException("the fake failed", "EncodingError"));
        }, 0);
      }
      const work = () => {
        if (this.state === "closed") return;
        for (let i = 0; i < n; i++) this.add_frame(left[i]);
        if (o.queue) { this.encodeQueueSize--; this.dispatchEvent(new Event("dequeue")); }
        if (!o.flush_only) this.later(() => this.deliver());
      };
      if (o.queue) setTimeout(work, o.hold_ms); else work();
    }
    later(f) { if (o.output_ms > 0) setTimeout(f, o.output_ms); else f(); }
    deliver() {
      while (this.ready.length > 0 && this.state !== "closed") {
        const bytes = this.ready.shift();
        this.rec.packets.push(bytes);
        this.since++;
        const chunk = { byteLength: bytes.length, type: "key", copyTo: (dst) => dst.set(bytes) };
        let meta = undefined;
        if (!this.sent_meta) {
          this.sent_meta = true;
          const d = this.index === 0 || o.main_description === undefined ? o.description : o.main_description;
          meta = d === null ? {} : { decoderConfig: { codec: "mp4a.40.2", description: bytes_of(d) } };
        }
        this.output(chunk, meta);
      }
    }
    async flush() {
      while (this.state !== "closed" && this.encodeQueueSize > 0) await new Promise((r) => setTimeout(r, 1));
      await new Promise((r) => setTimeout(r, o.output_ms + 1));
      if (this.state === "closed") throw new DOMException("the fake failed", "EncodingError");
      if (this.fill > 0) this.form();
      for (let i = 0; i < o.extra; i++) this.add_zeros(1024);
      this.deliver();
    }
    close() {
      if (this.state === "closed") throw invalid("already closed");
      this.state = "closed";
      stats.enc_closed++;
    }
  }
  const bytes_of = bytes;

  class FakeAudioDecoder {
    constructor(init) { this.output = init.output; this.state = "unconfigured"; this.count = 0; stats.dec_made++; }
    configure(config) {
      this.state = "configured";
      stats.decoder_description = Array.from(new Uint8Array(config.description));
    }
    decode() { this.count++; }
    async flush() {
      const n = this.count * 1024;
      const planar = new Float32Array(2 * n);
      const at = 4096 + (o.probe_delay === null ? o.delay : o.probe_delay);
      if (at >= 0 && at < n) { planar[at] = o.peak; planar[n + at] = o.peak; }
      this.output(new FakeAudioData({ numberOfFrames: n, sampleRate: 44100, timestamp: 0, data: planar }));
    }
    close() {
      if (this.state === "closed") throw invalid("already closed");
      this.state = "closed";
      stats.dec_closed++;
    }
  }

  class FakeEncodedAudioChunk { constructor(init) { Object.assign(this, init); } }

  globalThis.AudioEncoder = FakeAudioEncoder;
  globalThis.AudioDecoder = FakeAudioDecoder;
  globalThis.AudioData = FakeAudioData;
  globalThis.EncodedAudioChunk = FakeEncodedAudioChunk;
  fake = { saved, stats };
}
// Puts the browser's codecs back.
export function fake_aac_restore() {
  if (!fake) return;
  for (const [k, v] of Object.entries(fake.saved)) {
    if (v === undefined) delete globalThis[k]; else globalThis[k] = v;
  }
  fake = null;
}
export function fake_aac_stats() { return fake.stats; }
// The packets of encoder `i` (0 is the probe's), and the count of encoders.
export function fake_aac_packets(i) { return fake.stats.encoders[i].packets; }
export function fake_aac_encoders() { return fake.stats.encoders.length; }
export function fake_aac_input(i) {
  const r = fake.stats.encoders[i];
  return [new Float32Array(r.left), new Float32Array(r.right), new Float64Array(r.timestamps)];
}
"#)]
extern "C" {
    fn fake_aac_install(json: &str);
    fn fake_aac_restore();
    fn fake_aac_stats() -> JsValue;
    fn fake_aac_packets(i: u32) -> js_sys::Array;
    fn fake_aac_encoders() -> u32;
    fn fake_aac_input(i: u32) -> js_sys::Array;
}

/// The fake codecs, installed until dropped.
struct Fake;

impl Fake {
    fn new(options: &str) -> Self {
        fake_aac_restore();
        fake_aac_install(options);
        Fake
    }

    fn stat(&self, name: &str) -> f64 {
        field(&fake_aac_stats(), name).as_f64().unwrap_or(-1.0)
    }

    /// The packets of the encoder of the save: the last one made.
    fn packets(&self) -> Vec<Vec<u8>> {
        let last = fake_aac_encoders().saturating_sub(1);
        fake_aac_packets(last)
            .iter()
            .map(|p| Uint8Array::new(&p).to_vec())
            .collect()
    }

    /// The left and right samples and the timestamps that the encoder of the
    /// save got.
    fn input(&self) -> (Vec<f32>, Vec<f32>, Vec<f64>) {
        let last = fake_aac_encoders().saturating_sub(1);
        let input = fake_aac_input(last);
        let floats = |k: u32| Float32Array::new(&input.get(k)).to_vec();
        let times = js_sys::Float64Array::new(&input.get(2)).to_vec();
        (floats(0), floats(1), times)
    }

    /// Whether every `AudioData`, encoder and decoder is closed.
    fn all_closed(&self) -> Result<(), String> {
        let pairs = [
            ("AudioData", "data_made", "data_closed"),
            ("encoders", "enc_made", "enc_closed"),
            ("decoders", "dec_made", "dec_closed"),
        ];
        for (what, made, closed) in pairs {
            let (made, closed) = (self.stat(made), self.stat(closed));
            if made != closed {
                return Err(format!("{what}: {made} made, {closed} closed"));
            }
        }
        Ok(())
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        fake_aac_restore();
    }
}

/// Safari 27.2's description of AAC-LC at 44.1 kHz in stereo: a whole
/// `ES_Descriptor` around `12 10`.
const SAFARI_DESCRIPTION: [u8; 39] = [
    0x03, 0x80, 0x80, 0x80, 0x22, 0x00, 0x00, 0x00, 0x04, 0x80, 0x80, 0x80, 0x14, 0x40, 0x14, 0x00,
    0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x80, 0x80, 0x80, 0x02, 0x12,
    0x10, 0x06, 0x80, 0x80, 0x80, 0x01, 0x02,
];

/// The M4A of `packets`, as `m4a` makes it for an encode of `frames`
/// frames whose skip is `skip`: a 36-byte head, the packets, then `moov`.
fn m4a_of(packets: &[Vec<u8>], config: &[u8], skip: usize, frames: usize) -> Vec<u8> {
    let track = (packets.len() * 1024) as u64;
    let layout = m4a::Layout::new(
        m4a::Codec::Aac {
            channels: 2,
            config_len: config.len() as u8,
        },
        track,
        TONE_RATE,
        Some(m4a::Edit {
            skip: skip as u32,
            frames: frames as u64,
        }),
        m4a::Order::MoovLast,
    )
    .and_then(|layout| {
        layout.with_gapless(m4a::Gapless {
            delay: skip as u32,
            padding: (track - (skip + frames) as u64) as u32,
            frames: frames as u64,
        })
    })
    .expect("a layout");
    let sizes: Vec<u32> = packets.iter().map(|p| p.len() as u32).collect();
    let entry = m4a::SampleEntry::Aac {
        config: config.into(),
    };
    let head = layout.head(&entry, &sizes).expect("a head");
    assert_eq!(head.len(), 36, "the head of moov last is 36 bytes");
    let tail = layout.tail(&entry, &sizes).expect("a tail");
    [head, packets.concat(), tail].concat()
}

/// The encode of `left` and `right` as an AAC M4A on the memory path: the
/// result, and the file if it has one.
async fn encode_aac(left: &Float32Array, right: &Float32Array) -> Result<Vec<u8>, String> {
    encode(
        Codec::Aac,
        Some("tone.m4a".into()),
        arrays(left, right, Some(TONE_RATE)),
    )
    .await?;
    Ok(m4a_reply("aac").await)
}

/// The stored encode of a save's WAV of `frames` frames as an AAC M4A: the
/// reply's file, and the bytes of the output, if any.
async fn stored_aac(frames: usize) -> (Encoded, Option<Vec<u8>>, Float32Array, Float32Array) {
    let (wav, left, right) = save_wav(frames);
    plant_at(SAVE_WAV, &wav).await;
    let encoded = encode_stored(Codec::Aac, SAVE_WAV, frames, SAVE_M4A).await;
    assert!(encoded.is_ok(), "{frames} frames: {encoded:?}");
    (last_file("aac"), read_at(SAVE_M4A).await, left, right)
}

/// M4 of `plans/active/m4a.md`: an AAC save is the M4A of its packets.  Its
/// head is 36 bytes, the packets follow in order, then `moov`.  The skip of
/// its `elst` entry and of its `iTunSMPB` tag is the lead-in and the
/// encoder's delay.
#[wasm_bindgen_test]
async fn an_aac_is_the_m4a_of_its_packets() {
    for delay in [1024, 2112, 2624] {
        for frames in [1, 1023, 1024, 1025, 3 * 4096 + 7] {
            let case = format!("a delay of {delay}, {frames} frames");
            fresh().await;
            let fake = Fake::new(&format!(r#"{{"delay": {delay}}}"#));
            let (left, right) = (tone(220.0, frames), tone(330.0, frames));
            let file = encode_aac(&left, &right).await;
            assert!(file.is_ok(), "{case}: {file:?}");
            let file = file.expect("a file");
            let want = m4a_of(&fake.packets(), &[0x12, 0x10], LEAD_IN + delay, frames);
            assert!(
                file == want,
                "{case}, arrays: the M4A of the fake's packets"
            );
            assert!(
                file.windows(8).any(|w| w == b"iTunSMPB"),
                "{case}: an iTunSMPB tag"
            );
            drop(fake);

            fresh().await;
            let fake = Fake::new(&format!(r#"{{"delay": {delay}}}"#));
            let (encoded, stored, _, _) = stored_aac(frames).await;
            assert!(
                matches!(&encoded, Encoded::Stored { path } if path == SAVE_M4A),
                "{case}: {encoded:?}"
            );
            let want = m4a_of(&fake.packets(), &[0x12, 0x10], LEAD_IN + delay, frames);
            assert!(
                stored.as_deref() == Some(want.as_slice()),
                "{case}, stored: the M4A of the fake's packets"
            );
            assert_eq!(
                names_in("saves").await.as_deref(),
                Some("1792640000000-1.m4a")
            );
        }
    }
}

/// A request with no name gets the reply name `stem.m4a`, and the reply's
/// `Blob` has the type `audio/mp4`.
#[wasm_bindgen_test]
async fn an_aac_reply_is_an_m4a_named_for_its_extension() {
    fresh().await;
    let _fake = Fake::new("{}");
    let tone = tone(220.0, 5000);
    let result = encode(Codec::Aac, None, arrays(&tone, &tone, None)).await;
    assert!(result.is_ok(), "{result:?}");
    let reply = sent("aac").pop();
    assert!(reply.is_some(), "an aac reply");
    let reply = reply.expect("a reply");
    assert_eq!(
        field(&reply, "name").as_string().as_deref(),
        Some("stem.m4a")
    );
    let file: Blob = field(&reply, "file").dyn_into().expect("a Blob");
    assert_eq!(file.type_(), "audio/mp4");
}

/// The encoder gets `LEAD_IN` frames of silence, then the source's samples
/// in order, with timestamps in microseconds from the frame count.  On the
/// storage path, each sample is the WAV's `s / 32768`.
#[wasm_bindgen_test]
async fn an_aac_encodes_the_lead_in_then_the_source() {
    let frames = 2 * AAC_BLOCK + 5;
    for stored in [false, true] {
        fresh().await;
        let fake = Fake::new("{}");
        let (left, right) = if stored {
            let (_, _, left, right) = stored_aac(frames).await;
            (left, right)
        } else {
            let (left, right) = (tone(220.0, frames), tone(330.0, frames));
            let file = encode_aac(&left, &right).await;
            assert!(file.is_ok(), "{file:?}");
            (left, right)
        };
        let (got_left, got_right, times) = fake.input();
        let silence = vec![0.0; LEAD_IN];
        let want = |c: &Float32Array| [silence.clone(), c.to_vec()].concat();
        assert!(got_left == want(&left), "stored {stored}: the left channel");
        assert!(
            got_right == want(&right),
            "stored {stored}: the right channel"
        );
        let at = |frame: usize| (frame as f64 * 1e6 / f64::from(TONE_RATE)).round();
        assert_eq!(
            times,
            [
                0.0,
                at(LEAD_IN),
                at(LEAD_IN + AAC_BLOCK),
                at(LEAD_IN + 2 * AAC_BLOCK)
            ],
            "stored {stored}: the timestamps"
        );
    }
}

/// The encode waits for the encoder's queue, so the queue never holds more
/// than `QUEUE_MOST` blocks and the one just sent.
#[wasm_bindgen_test]
async fn an_aac_waits_for_the_encoders_queue() {
    fresh().await;
    let fake = Fake::new(r#"{"hold_ms": 2, "record": false}"#);
    let frames = 5 * TONE_RATE as usize;
    let (left, right) = (tone(220.0, frames), tone(330.0, frames));
    let file = encode_aac(&left, &right).await;
    assert!(file.is_ok(), "{file:?}");
    let most = fake.stat("max_queue");
    assert!(
        most <= f64::from(QUEUE_MOST + 1),
        "MEM-4: the encoder's queue held {most} blocks, more than {}.  Fix: wait for \
         `dequeue` while encodeQueueSize is above QUEUE_MOST.",
        QUEUE_MOST + 1
    );
}

/// The encode waits for the encoder's output, so the frames sent and not
/// yet back stay below `OUT_MOST` and a block.  The packets that come
/// between two blocks stay below a fixed count.  An encoder that gives no
/// output before `flush` ends the encode with a clear text.
#[wasm_bindgen_test]
async fn an_aac_waits_for_the_encoders_output() {
    fresh().await;
    let fake = Fake::new(r#"{"queue": false, "output_ms": 3, "record": false}"#);
    let frames = 10 * TONE_RATE as usize;
    let (left, right) = (tone(220.0, frames), tone(330.0, frames));
    let file = encode_aac(&left, &right).await;
    assert!(file.is_ok(), "{file:?}");
    let (lag, burst) = (fake.stat("max_lag"), fake.stat("max_burst"));
    web_sys::console::log_1(
        &format!("an AAC of 10 s: a lag of at most {lag} frames, at most {burst} packets at once")
            .into(),
    );
    assert!(
        lag <= (OUT_MOST + AAC_BLOCK) as f64,
        "MEM-4: {lag} frames were sent and not back, more than {}.  Fix: wait for an output \
         while more than OUT_MOST frames are out.",
        OUT_MOST + AAC_BLOCK
    );
    let most_packets = 48.0;
    assert!(
        burst < most_packets,
        "MEM-4: {burst} packets came between two blocks, not fewer than {most_packets}"
    );
    drop(fake);

    fresh().await;
    let fake = Fake::new(r#"{"queue": false, "flush_only": true, "record": false}"#);
    set_output_wait_ms(200);
    let file = encode_aac(&left, &right).await;
    set_output_wait_ms(10_000);
    assert!(file.is_err(), "{file:?}");
    assert_eq!(file.unwrap_err(), "AAC encoder: the encoder gave no output");
    assert_eq!(fake.all_closed(), Ok(()));
}

/// Safari gives a whole `ES_Descriptor` as its description.  The file's
/// `esds` holds the bare config `12 10`, and the probe decodes with it.
#[wasm_bindgen_test]
async fn an_aac_takes_the_config_out_of_safaris_description() {
    fresh().await;
    let fake = Fake::new(&format!(r#"{{"description": {SAFARI_DESCRIPTION:?}}}"#));
    let frames = 3000;
    let (left, right) = (tone(220.0, frames), tone(330.0, frames));
    let file = encode_aac(&left, &right).await;
    assert!(file.is_ok(), "{file:?}");
    let want = m4a_of(&fake.packets(), &[0x12, 0x10], LEAD_IN + 2112, frames);
    assert!(
        file.expect("a file") == want,
        "the esds holds the bare config"
    );
    let decoder = field(&fake_aac_stats(), "decoder_description");
    let decoder: Vec<u8> = js_sys::Array::from(&decoder)
        .iter()
        .map(|b| b.as_f64().unwrap_or(-1.0) as u8)
        .collect();
    assert_eq!(
        decoder,
        [0x12, 0x10],
        "the probe decodes with the bare config"
    );
}

/// Each refusal of an AAC encode: its fake options, its rate, its frames,
/// and the start of its text.
fn refusals() -> Vec<(&'static str, String, u32, usize, String)> {
    let rest = |what: &str| format!("AAC encoder: {what}");
    vec![
        (
            "no samples",
            "{}".into(),
            TONE_RATE,
            0,
            rest("the stem has no samples"),
        ),
        (
            "no AAC encoder",
            r#"{"supported": false}"#.into(),
            TONE_RATE,
            3000,
            rest("this browser has no AAC encoder"),
        ),
        (
            "no peak",
            r#"{"peak": 0.1}"#.into(),
            TONE_RATE,
            3000,
            rest("the probe's impulse decoded with a peak of 0.1"),
        ),
        (
            "a delay of 8192",
            r#"{"delay": 8192}"#.into(),
            TONE_RATE,
            3000,
            rest("the probe gave a delay of 8192 frames, not 0 to 8191"),
        ),
        (
            "a delay below 0",
            r#"{"probe_delay": -96}"#.into(),
            TONE_RATE,
            3000,
            rest("the probe gave a delay of -96 frames, not 0 to 8191"),
        ),
        (
            "an encoder error",
            r#"{"error_after": 8192}"#.into(),
            TONE_RATE,
            10 * AAC_BLOCK,
            rest("EncodingError: the fake failed"),
        ),
        (
            "HE-AAC",
            r#"{"main_description": [43, 146, 8, 0]}"#.into(),
            TONE_RATE,
            3000,
            rest("the encoder gave object type 5, not 2 (AAC-LC)"),
        ),
        (
            "1 channel",
            r#"{"main_description": [18, 8]}"#.into(),
            TONE_RATE,
            3000,
            rest("the encoder gave channel configuration 1, not 2 (stereo)"),
        ),
        (
            "no description",
            r#"{"main_description": null}"#.into(),
            TONE_RATE,
            3000,
            rest("the encoder gave no description"),
        ),
        (
            "22,000 Hz",
            "{}".into(),
            22_000,
            3000,
            rest("22000 Hz is not a rate of the AAC table"),
        ),
        (
            "960 frames",
            r#"{"main_description": [18, 20]}"#.into(),
            TONE_RATE,
            3000,
            rest("the encoder gave 960 frames in each packet, not 1024"),
        ),
        (
            "too few packets",
            r#"{"delay": 0, "probe_delay": 4000, "extra": 0}"#.into(),
            TONE_RATE,
            1,
            rest("the encoder gave too few packets"),
        ),
    ]
}

/// Each refusal has its own text on the memory path, and leaves no
/// `AudioData`, encoder or decoder open.
#[wasm_bindgen_test]
async fn each_aac_refusal_says_why() {
    for (case, options, rate, frames, text) in refusals() {
        fresh().await;
        let fake = Fake::new(&options);
        let (left, right) = (tone(220.0, frames), tone(330.0, frames));
        let result = encode(Codec::Aac, None, arrays(&left, &right, Some(rate))).await;
        assert!(result.is_err(), "{case}: {result:?}");
        let error = result.unwrap_err();
        assert!(error.starts_with(&text), "{case}: {error}");
        assert!(sent("aac").is_empty(), "{case}: no reply of a file");
        assert_eq!(fake.all_closed(), Ok(()), "{case}");
    }
}

/// On the storage path, each refusal is a `failed` reply that names the
/// encoder, and leaves no output file and no WAV.
#[wasm_bindgen_test]
async fn a_refused_stored_aac_is_an_encoder_failure_with_no_file() {
    for (case, options, rate, frames, text) in refusals() {
        fresh().await;
        let fake = Fake::new(&options);
        let (wav, _, _) = save_wav(frames);
        plant_at(SAVE_WAV, &wav).await;
        let encoded = encode(
            Codec::Aac,
            None,
            Encode::Stored {
                wav: SAVE_WAV.into(),
                frames,
                out: SAVE_M4A.into(),
                rate,
            },
        )
        .await;
        assert!(encoded.is_ok(), "{case}: an error reply: {encoded:?}");
        match last_file("aac") {
            Encoded::Failed {
                path,
                text: got,
                in_encoder,
            } => {
                assert_eq!(path, SAVE_M4A, "{case}");
                assert!(got.starts_with(&text), "{case}: {got}");
                assert!(in_encoder, "{case}: a failure of the encoder");
            }
            other => panic!("{case}: {other:?}"),
        }
        assert_eq!(names_in("saves").await.as_deref(), Some(""), "{case}");
        assert_eq!(fake.all_closed(), Ok(()), "{case}");
    }
}

/// A refused write of a stored AAC is a failure of browser storage, not of
/// the encoder, and leaves no file.
#[wasm_bindgen_test]
async fn a_refused_write_of_a_stored_aac_is_not_an_encoder_failure() {
    for fail_at in [1, 2] {
        fresh().await;
        let fake = Fake::new("{}");
        let frames = 3 * AAC_BLOCK;
        let (wav, _, _) = save_wav(frames);
        plant_at(SAVE_WAV, &wav).await;
        start_sync_spy(fail_at);
        let encoded = encode_stored(Codec::Aac, SAVE_WAV, frames, SAVE_M4A).await;
        stop_sync_spy();
        assert!(encoded.is_ok(), "write {fail_at}: {encoded:?}");
        match last_file("aac") {
            Encoded::Failed {
                in_encoder, text, ..
            } => {
                assert!(!in_encoder, "write {fail_at}: {text}");
                assert!(
                    text.contains("could not be written"),
                    "write {fail_at}: {text}"
                );
            }
            other => panic!("write {fail_at}: {other:?}"),
        }
        assert_eq!(names_in("saves").await.as_deref(), Some(""));
        assert_eq!(fake.all_closed(), Ok(()), "write {fail_at}");
    }
}

/// MEM-4: an AAC encode closes each `AudioData`, encoder and decoder, after
/// an encode and after an encoder error in the middle of one.
#[wasm_bindgen_test]
async fn an_aac_closes_each_audio_data() {
    for (case, options) in [
        ("an encode", "{}"),
        ("an error in the middle", r#"{"error_after": 16384}"#),
    ] {
        fresh().await;
        let fake = Fake::new(options);
        let frames = 10 * AAC_BLOCK;
        let (left, right) = (tone(220.0, frames), tone(330.0, frames));
        let _ = encode_aac(&left, &right).await;
        // The probe's, the lead-in's and at least one block's.
        assert!(fake.stat("data_made") >= 3.0, "{case}");
        assert_eq!(
            fake.all_closed(),
            Ok(()),
            "MEM-4: {case}.  Fix: close each AudioData after its encode, and each codec at the end."
        );
    }
}

/// MEM-5: a stored AAC reserves its 36-byte head at 0, writes each packet
/// as it comes after the reads, then `moov`, then the head at 0.  No write
/// is a view.
#[wasm_bindgen_test]
async fn a_stored_aac_writes_whole_arrays_in_order() {
    fresh().await;
    let fake = Fake::new("{}");
    let frames = 3 * AAC_BLOCK + 7;
    let (wav, _, _) = save_wav(frames);
    plant_at(SAVE_WAV, &wav).await;
    start_order_spy();
    let encoded = encode_stored(Codec::Aac, SAVE_WAV, frames, SAVE_M4A).await;
    let record = stop_order_spy();
    assert!(encoded.is_ok(), "{encoded:?}");
    assert!(matches!(last_file("aac"), Encoded::Stored { .. }));
    assert!(!record.contains("view"), "MEM-5: {record}");
    let steps: Vec<&str> = record.split(' ').collect();
    let block = format!("r{}", 4 * AAC_BLOCK);
    let reads: Vec<&str> = steps
        .iter()
        .copied()
        .filter(|s| s.starts_with('r'))
        .collect();
    assert_eq!(reads, [block.as_str(), &block, &block, "r28"], "{record}");
    let packets = fake.packets();
    let writes: Vec<(usize, usize)> = steps
        .iter()
        .filter_map(|s| s.strip_prefix('w'))
        .map(|w| {
            let (bytes, at) = w.split_once('@').expect("a write");
            (
                bytes.parse().expect("a size"),
                at.parse().expect("an offset"),
            )
        })
        .collect();
    assert_eq!(writes.len(), packets.len() + 3, "{record}");
    assert_eq!(writes[0], (36, 0), "the reserve first: {record}");
    assert_eq!(writes[writes.len() - 1], (36, 0), "the head last: {record}");
    let mut at = 36;
    for (i, packet) in packets.iter().enumerate() {
        assert_eq!(writes[1 + i], (packet.len(), at), "packet {i}: {record}");
        at += packet.len();
    }
    assert_eq!(
        writes[writes.len() - 2].1,
        at,
        "moov after the packets: {record}"
    );
    assert!(
        steps[1].starts_with('r'),
        "the first read comes before the first packet: {record}"
    );
}

/// The packets of the encoder of the save, from the fake.
fn packets_of_the_save() -> usize {
    let last = fake_aac_encoders().saturating_sub(1);
    fake_aac_packets(last).length() as usize
}

/// The growth of an AAC encode's peak from 10 s to 60 s, on `path`, is the
/// packet table only (Step 3 of `plans/active/m4a-m4.md`).  At 10 s and
/// 60 s the peak is in the loop, where only the sizes grow: 4 bytes for
/// each packet, with a fixed slack of 1 KiB.
fn assert_aac_table_growth(
    path: &str,
    (ten, ten_packets): (usize, usize),
    (sixty, sixty_packets): (usize, usize),
) {
    let more = sixty_packets - ten_packets;
    let slack = 1024;
    let growth = sixty.saturating_sub(ten);
    web_sys::console::log_1(
        &format!(
            "an AAC of 10 s and 60 s on the {path} path: peaks of {ten} and {sixty} bytes, \
             {growth} bytes more for {more} more packets"
        )
        .into(),
    );
    assert!(
        growth <= 4 * more + slack,
        "MEM-1: on the {path} path, an AAC of 60 s held {growth} bytes more than one of 10 s, \
         more than 4 bytes for each of {more} more packets and {slack}.  Fix: keep only the \
         packet sizes, made with `Vec::with_capacity`, and put each packet as a JS array."
    );
}

/// MEM-1 and MEM-4: a stored AAC encode holds its packet table, and no
/// more, as the song grows.
#[wasm_bindgen_test]
async fn a_stored_aac_holds_only_its_packet_table() {
    let _fake = Fake::new(r#"{"record": false}"#);
    let (ten, _) = stored_peak(Codec::Aac, 10).await;
    let ten_packets = packets_of_the_save();
    let (sixty, _) = stored_peak(Codec::Aac, 60).await;
    let sixty_packets = packets_of_the_save();
    assert_aac_table_growth("storage", (ten, ten_packets), (sixty, sixty_packets));
    let heap = js_sys::Reflect::get(&js_sys::global(), &"performance".into())
        .and_then(|p| js_sys::Reflect::get(&p, &"memory".into()))
        .map(|m| !m.is_undefined())
        .unwrap_or(false);
    web_sys::console::log_1(&format!("the worker gives performance.memory: {heap}").into());
}

/// MEM-1 and MEM-4: an AAC encode on the memory path holds its packet
/// table, and no more, as the song grows.  JS holds the input and the
/// parts.
#[wasm_bindgen_test]
async fn an_aac_is_encoded_in_blocks() {
    let _fake = Fake::new(r#"{"record": false}"#);
    let ten = arrays_peak(Codec::Aac, 10).await;
    let ten_packets = packets_of_the_save();
    let sixty = arrays_peak(Codec::Aac, 60).await;
    let sixty_packets = packets_of_the_save();
    assert_aac_table_growth("memory", (ten, ten_packets), (sixty, sixty_packets));
}

/// The end of an AAC encode holds only the `moov` beside the packet table,
/// on each sink: at most 12 bytes for each packet with the sizes, and a
/// slack of 1 KiB.  The layout has 20 minutes, 51,679 packets, where the
/// end is the peak.  The test makes the sizes, so it encodes nothing.
#[wasm_bindgen_test]
async fn the_end_of_an_aac_holds_only_its_moov() {
    fresh().await;
    let packets = 20 * 60 * TONE_RATE as usize / 1024;
    let frames = packets * 1024 - LEAD_IN - 2112;
    let config = [0x12, 0x10];
    let layout = m4a::Layout::new(
        m4a::Codec::Aac {
            channels: 2,
            config_len: 2,
        },
        (packets * 1024) as u64,
        TONE_RATE,
        Some(m4a::Edit {
            skip: (LEAD_IN + 2112) as u32,
            frames: frames as u64,
        }),
        m4a::Order::MoovLast,
    )
    .and_then(|l| {
        l.with_gapless(m4a::Gapless {
            delay: (LEAD_IN + 2112) as u32,
            padding: 0,
            frames: frames as u64,
        })
    })
    .expect("a layout");
    let sizes = vec![700_u32; packets];
    let slack = 1024;
    assert!(
        4 * packets + layout.tail_len() <= 12 * packets + slack,
        "the sizes and the moov are at most 12 bytes for each packet"
    );
    for mut out in [file_sink().await, EncodeOut::Parts(js_sys::Array::new())] {
        out.reserve(36).await.expect("reserved");
        let before = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(before));
        let ended = crate::web::aac::aac_end(&layout, &sizes, config.into(), 36, &mut out).await;
        let held = PEAK.with(Cell::get) - before;
        assert!(ended.is_ok(), "{ended:?}");
        let what = match &out {
            EncodeOut::File(_) => "file",
            EncodeOut::Parts(_) => "parts",
        };
        web_sys::console::log_1(
            &format!("the end of an AAC of {packets} packets on the {what} sink held {held} bytes")
                .into(),
        );
        assert!(
            held <= layout.tail_len() + slack,
            "MEM-1: the end of an AAC of {packets} packets on the {what} sink held {held} bytes, \
             more than its moov of {} bytes and {slack}.  Fix: make the moov in one buffer of \
             `tail_len()`, and drop it before the head.",
            layout.tail_len()
        );
        if let EncodeOut::File(file) = out {
            file.close().await.expect("closed");
        }
    }
}
