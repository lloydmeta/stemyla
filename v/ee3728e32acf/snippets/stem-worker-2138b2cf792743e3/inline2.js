
// Whether this browser has an AAC encoder for `config`.
export async function aac_supported(config) {
  if (typeof AudioEncoder === "undefined") return false;
  try { return !!(await AudioEncoder.isConfigSupported(config)).supported; }
  catch (_) { return false; }
}
// One encode: its encoder, its queue of packets, the description of its
// first output, its error, and the function that ends a wait.
export function aac_open(config) {
  const s = { enc: null, chunks: [], received: 0, description: null, error: null, wake: null };
  const wake = () => { const w = s.wake; s.wake = null; if (w) w(); };
  s.enc = new AudioEncoder({
    output: (chunk, meta) => {
      const bytes = new Uint8Array(chunk.byteLength);
      chunk.copyTo(bytes);
      s.chunks.push(bytes);
      s.received += 1;
      const d = meta && meta.decoderConfig && meta.decoderConfig.description;
      if (d && !s.description) {
        const view = ArrayBuffer.isView(d) ? new Uint8Array(d.buffer, d.byteOffset, d.byteLength) : new Uint8Array(d);
        s.description = view.slice();
      }
      wake();
    },
    error: (e) => { if (!s.error) s.error = String(e); wake(); },
  });
  try { s.enc.configure(config); }
  catch (e) { aac_close(s); throw e; }
  return s;
}
// Encodes `frames` frames of `planar` (the left plane, then the right).
// The AudioData is closed at once: the encoder keeps its own copy.
export function aac_encode(s, planar, frames, rate, timestamp) {
  if (s.error) throw new Error(s.error);
  const data = new AudioData({ format: "f32-planar", sampleRate: rate, numberOfFrames: frames,
    numberOfChannels: 2, timestamp, data: planar });
  try { s.enc.encode(data); } finally { data.close(); }
}
// Encodes `frames` frames of silence, made in JS.
export function aac_encode_silence(s, frames, rate, timestamp) {
  aac_encode(s, new Float32Array(2 * frames), frames, rate, timestamp);
}
// Waits while the encoder's queue holds more than `most` blocks, or until
// an error: whether the queue moved within `ms` at each wait.  The listener
// is added before the queue is read again, so the wait cannot miss the
// event.
export async function aac_wait_queue(s, most, ms) {
  while (!s.error && s.enc.encodeQueueSize > most) {
    const moved = await new Promise((r) => {
      let timer = null;
      const done = (ok) => {
        clearTimeout(timer);
        s.enc.removeEventListener("dequeue", on);
        s.wake = null;
        r(ok);
      };
      const on = () => done(true);
      s.wake = on;
      s.enc.addEventListener("dequeue", on);
      if (s.error || s.enc.encodeQueueSize <= most) { done(true); return; }
      timer = setTimeout(() => done(false), ms);
    });
    if (!moved) return false;
  }
  return true;
}
// Waits for the next output or error: whether one came within `ms`.  An
// error that came before the wait ends it at once.
export function aac_wait_output(s, ms) {
  const seen = s.received;
  if (s.error) return Promise.resolve(true);
  return new Promise((r) => {
    let timer = null;
    const done = () => { clearTimeout(timer); s.wake = null; r(s.received > seen || !!s.error); };
    s.wake = done;
    timer = setTimeout(done, ms);
  });
}
// Yields one task, so that the outputs of the encoder can arrive.  A
// message is a task with no clamp, as a nested setTimeout has.
export function aac_yield() {
  return new Promise((r) => {
    const ch = new MessageChannel();
    ch.port1.onmessage = () => { ch.port1.close(); r(); };
    ch.port2.postMessage(0);
  });
}
// The packets that came since the last take, as an array of Uint8Arrays.
export function aac_take(s) { const c = s.chunks; s.chunks = []; return c; }
export function aac_received(s) { return s.received; }
export function aac_error(s) { return s.error; }
export function aac_description(s) { return s.description; }
// `promise`, or a rejection with the text `late` when it does not end
// within `ms`.  A later rejection of `promise` is ignored.
function within(promise, ms, late) {
  promise.catch(() => {});
  let timer = null;
  const timeout = new Promise((_, no) => { timer = setTimeout(() => no(late), ms); });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}
// The flush of the encoder, within `ms`.
export async function aac_flush(s, ms) {
  if (s.error) throw new Error(s.error);
  try { await within(s.enc.flush(), ms, "the encoder's flush did not end"); }
  catch (e) {
    if (s.error) throw new Error(s.error);
    throw typeof e === "string" ? e : new Error(String(e));
  }
  if (s.error) throw new Error(s.error);
}
// Closes the encoder, if it is not closed.  After its error callback, a
// codec is closed, and a second close throws.
export function aac_close(s) {
  if (s.enc && s.enc.state !== "closed") s.enc.close();
}
// The probe: encodes 16384 frames with 1.0 at frame 4096 and 0 elsewhere,
// and gives the packets and the description.
export async function aac_probe(config, ms) {
  const s = aac_open(config);
  try {
    const frames = 16384;
    const planar = new Float32Array(2 * frames);
    planar[4096] = 1;
    planar[frames + 4096] = 1;
    aac_encode(s, planar, frames, config.sampleRate, 0);
    await aac_flush(s, ms);
    return [aac_take(s), s.description];
  } finally { aac_close(s); }
}
// Decodes `chunks` with `AudioDecoder` and the bare AudioSpecificConfig
// `asc`: the frame and the value of the largest absolute sample of the
// left channel, and the frames decoded.
export async function aac_peak(config, asc, chunks, ms) {
  let error = null, frames = 0, at = -1, peak = 0;
  const dec = new AudioDecoder({
    output: (a) => {
      try {
        const left = new Float32Array(a.numberOfFrames);
        a.copyTo(left, { planeIndex: 0, format: "f32-planar" });
        for (let i = 0; i < left.length; i++) {
          if (Math.abs(left[i]) > peak) { peak = Math.abs(left[i]); at = frames + i; }
        }
        frames += left.length;
      } finally { a.close(); }
    },
    error: (e) => { if (!error) error = String(e); },
  });
  try {
    dec.configure({ codec: config.codec, sampleRate: config.sampleRate,
      numberOfChannels: config.numberOfChannels, description: asc });
    chunks.forEach((data, k) => dec.decode(new EncodedAudioChunk({ type: "key",
      timestamp: Math.round(k * 1024 * 1e6 / config.sampleRate), data })));
    await within(dec.flush(), ms, "the decoder's flush did not end");
  } catch (e) {
    if (error) throw new Error(error);
    throw typeof e === "string" ? e : new Error(String(e));
  }
  finally { if (dec.state !== "closed") dec.close(); }
  if (error) throw new Error(error);
  return [at, peak, frames];
}
