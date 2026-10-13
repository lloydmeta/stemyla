"""Export BS-RoFormer SW for the Rust port.

1. Converts the PyTorch checkpoint to safetensors (f32 and f16), keeping the
   original parameter names.  The 24 copies of the rotary frequencies are
   checked against 1 / 10000^(2i/64) and dropped, since the Rust side
   computes them.
2. Writes reference fixtures for the Rust tests:
   * stft.safetensors: torch.stft of a 1 s stereo clip and the
     torch.istft round trip.
   * forward.safetensors: activations of one forward pass on that clip
     (band split, first time and frequency transformer, every layer
     output, final norm, masks, separated audio).
   * demix.safetensors: demix_track with a small chunk size and a cheap
     deterministic stand-in model, to check chunking and overlap-add
     separately from the network.
   * web/clip.f32 and web/clip_ref.f32: a 0.5 s stereo clip and the real
     network's demix_track output for it with 0.37 s chunks (33 STFT
     frames; raw little-endian f32, [2, N] and [6, 2, N]), for the
     in-browser check (tools/worker_check.mjs).  Short, because WebGPU on
     SwiftShader (the CPU) runs the network at well under 1 GFLOP/s.

The checkpoint is loaded with torch.load(weights_only=True) and strictly.

Usage: python -I roformer_export.py <checkpoint.ckpt> <clip.wav> <out_dir>
"""

import os
import sys
from pathlib import Path

import numpy as np
import torch
import yaml
from ml_collections import ConfigDict
from safetensors.torch import save_file

import bs_roformer
from bs_roformer.utils import demix_track, get_model_from_config

RATE = 44100


def read_wav_f32(path):
    d = Path(path).read_bytes()
    i = d.find(b"data")
    n = int.from_bytes(d[i + 4 : i + 8], "little")
    return np.frombuffer(d[i + 8 : i + 8 + n], dtype="<f4").reshape(-1, 2).T.copy()


def load_model(ckpt):
    cfg_path = Path(bs_roformer.__file__).parent / "configs/BS-Rofo-SW-Fixed.yaml"
    config = ConfigDict(yaml.safe_load(cfg_path.read_text()))
    model = get_model_from_config("bs_roformer", config)
    state = torch.load(ckpt, map_location="cpu", weights_only=True)
    model.load_state_dict(state, strict=True)
    model.eval()
    return config, model, state


def export_weights(state, out):
    expected = 1.0 / (10000 ** (torch.arange(0, 64, 2).float() / 64))
    weights = {}
    for k, v in state.items():
        if k.endswith("rotary_embed.freqs"):
            err = (v - expected).abs().max().item()
            assert err < 1e-6, f"{k} differs from the default rotary frequencies by {err}"
            continue
        weights[k] = v.contiguous()
    meta = {
        "model": "BS-RoFormer SW (BS-Rofo-SW-Fixed)",
        "instruments": "bass,drums,other,vocals,guitar,piano",
        "source_sha256": "24e7d35ee9c64415673d3fd33e06a67cac2c103c5df6267ba1576459c775916e",
    }
    save_file(weights, str(out / "bs_roformer_sw.f32.safetensors"), metadata=meta)
    half = {k: v.half() for k, v in weights.items()}
    save_file(half, str(out / "bs_roformer_sw.f16.safetensors"), metadata=meta)
    n = sum(v.numel() for v in weights.values())
    print(f"weights: {len(weights)} tensors, {n} parameters")


def stft_fixture(model, clip, out):
    window = torch.hann_window(2048)
    spec = torch.stft(clip, n_fft=2048, hop_length=512, win_length=2048, window=window,
                      normalized=False, return_complex=True)
    back = torch.istft(spec, n_fft=2048, hop_length=512, win_length=2048, window=window,
                       normalized=False, return_complex=False)
    save_file({
        "audio": clip.contiguous(),
        "spec": torch.view_as_real(spec).contiguous(),  # [S, F, T, 2]
        "istft": back.contiguous(),
    }, str(out / "stft.safetensors"))


def forward_fixture(model, clip, out):
    acts = {}

    def keep(name, reshape=None):
        def hook(_m, _inp, output):
            x = output[0] if isinstance(output, tuple) else output
            acts[name] = (reshape(x) if reshape else x).detach().clone()
        return hook

    frames = clip.shape[1] // 512 + 1
    bands = len(model.band_split.dim_inputs)
    # time transformer output is packed as [(b f), t, d]; store as [t, f, d]
    as_tfd_from_time = lambda x: x.reshape(bands, frames, -1).transpose(0, 1)
    # frequency transformer output is packed as [(b t), f, d]
    as_tfd_from_freq = lambda x: x.reshape(frames, bands, -1)

    hooks = [
        model.band_split.register_forward_hook(keep("band_split", lambda x: x[0])),
        model.final_norm.register_forward_hook(keep("final_norm", lambda x: x[0])),
    ]
    for i, (tt, ft) in enumerate(model.layers):
        hooks.append(tt.register_forward_hook(keep(f"layer{i}.time", as_tfd_from_time)))
        hooks.append(ft.register_forward_hook(keep(f"layer{i}.freq", as_tfd_from_freq)))
    for n, me in enumerate(model.mask_estimators):
        hooks.append(me.register_forward_hook(keep(f"mask{n}", lambda x: x[0])))

    with torch.no_grad():
        audio = model(clip.unsqueeze(0))[0]
    for h in hooks:
        h.remove()
    acts["input"] = clip.contiguous()
    acts["output"] = audio.contiguous()
    save_file({k: v.contiguous() for k, v in acts.items()}, str(out / "forward.safetensors"))
    print(f"forward: frames {frames}, bands {bands}, output {tuple(audio.shape)}")


def stand_in(part):
    """Cheap deterministic stand-in for the network: [1, 2, C] -> [1, 6, 2, C]."""
    x = part[0]
    stems = [torch.tanh(x * (k + 1) * 0.5) + 0.01 * k for k in range(6)]
    return torch.stack(stems).unsqueeze(0)


def demix_fixture(config, out):
    cases = {}
    rng = np.random.default_rng(7)
    # (chunk, overlap, length): reflect-padded long input with a short reflect-padded
    # last chunk; a long input whose last chunk is zero-padded; an input too short
    # for the border padding (single chunk).
    for name, c, n, length in [("a", 1000, 2, 4321), ("b", 1000, 2, 5100), ("c", 1000, 4, 2900),
                               ("d", 1000, 2, 900)]:
        cfg = ConfigDict(config.to_dict())
        cfg.inference.chunk_size = c
        cfg.inference.num_overlap = n
        mix = torch.from_numpy(rng.standard_normal((2, length)).astype(np.float32) * 0.5)
        res = demix_track(cfg, stand_in, mix, torch.device("cpu"))
        if isinstance(res, tuple):
            res = res[0]
        stems = np.stack([np.asarray(res[k]) for k in cfg.training.instruments])
        cases[f"{name}.mix"] = mix
        cases[f"{name}.out"] = torch.from_numpy(stems.astype(np.float32))
        cases[f"{name}.params"] = torch.tensor([c, n, length], dtype=torch.int64)
    save_file({k: v.contiguous() for k, v in cases.items()}, str(out / "demix.safetensors"))
    print(f"demix: {len(cases) // 3} cases")


def web_fixture(config, model, mix, out):
    clip = mix[:, 5 * RATE : 5 * RATE + RATE // 2].contiguous()
    cfg = ConfigDict(config.to_dict())
    cfg.inference.chunk_size = 32 * 512
    cfg.inference.num_overlap = 2
    with torch.no_grad():
        res = demix_track(cfg, model, clip, torch.device("cpu"))
    if isinstance(res, tuple):
        res = res[0]
    stems = np.stack([np.asarray(res[k]) for k in cfg.training.instruments]).astype("<f4")
    web = out / "web"
    web.mkdir(exist_ok=True)
    (web / "clip.f32").write_bytes(clip.numpy().astype("<f4").tobytes())
    (web / "clip_ref.f32").write_bytes(stems.tobytes())
    print(f"web: clip {tuple(clip.shape)}, reference {stems.shape}")


def main():
    ckpt, wav, out = sys.argv[1:4]
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    torch.set_num_threads(os.cpu_count() or 2)
    torch.manual_seed(0)
    config, model, state = load_model(ckpt)
    export_weights(state, out)
    mix = torch.from_numpy(read_wav_f32(wav))
    clip = mix[:, 5 * RATE : 6 * RATE].contiguous()  # 1 s from 0:05 of the excerpt
    stft_fixture(model, clip, out)
    forward_fixture(model, clip, out)
    demix_fixture(config, out)
    web_fixture(config, model, mix, out)


if __name__ == "__main__":
    main()
