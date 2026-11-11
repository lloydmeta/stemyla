
// Whether this browser can encode AAC with `config`, as [ok, why].  Each
// failure gives false, with no throw.  It closes its AudioData and its
// encoder on each path.
export async function aac_probe(config, ms) {
  if (typeof AudioEncoder === "undefined") return [false, "no AudioEncoder"];
  try {
    const support = await AudioEncoder.isConfigSupported(config);
    if (!support || !support.supported) return [false, "isConfigSupported is false"];
  } catch (e) { return [false, `isConfigSupported: ${e}`]; }
  let enc = null, data = null, packets = 0, error = null, timer = null;
  try {
    enc = new AudioEncoder({ output: () => { packets++; }, error: (e) => { error = String(e); } });
    enc.configure(config);
    const frames = 4096;
    data = new AudioData({ format: "f32-planar", sampleRate: config.sampleRate, numberOfFrames: frames,
      numberOfChannels: config.numberOfChannels, timestamp: 0,
      data: new Float32Array(frames * config.numberOfChannels) });
    enc.encode(data);
    data.close();
    data = null;
    const flushed = enc.flush().then(() => "flushed", (e) => `flush: ${error || e}`);
    const late = new Promise((r) => { timer = setTimeout(() => r("late"), ms); });
    const end = await Promise.race([flushed, late]);
    if (end === "late") return [false, `no packet within ${ms} ms`];
    if (end !== "flushed") return [false, end];
    if (error) return [false, error];
    return packets > 0 ? [true, ""] : [false, "the test encode gave no packet"];
  } catch (e) { return [false, error || String(e)]; }
  finally {
    clearTimeout(timer);
    if (data) data.close();
    if (enc && enc.state !== "closed") enc.close();
  }
}
