---
paths:
  - "protocol/**"
  - "worker/src/web.rs"
  - "worker/src/web/**"
  - "app/src/separator.rs"
  - "decoder/src/**"
  - "app/src/decoder.rs"
  - "app/src/decoder/**"
  - "xtask/src/e2e.rs"
  - "xtask/src/e2e/**"
  - "xtask/e2e/**"
---

# The page <-> worker wire format

LAYER-1, PROTO-1 and PROTO-2 in `docs/invariants.md`.  The format is documented at the top of `protocol/src/lib.rs`.

* Change a message in `stem-protocol` only, with its encoder and decoder together, and extend `protocol/tests/web.rs` (round trip through a structured clone with its transfers).
* `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` and `xtask/src/e2e/` write and read messages by hand: change them in the same commit.  CI's `browser-tests` job runs `e2e storage` and `e2e flac`, which write stem worker messages by hand.  `stored-stems`, `worker-check` and `weights` also write them, but CI does not run them (TD-16).  They need a GPU and the weights.  When you change the messages of one of them, ask a person to run it on a GPU with the weights.  Name the run in the PR.  Else run a scenario by hand only when CI failed it (decision 0026).  `e2e_messages_use_protocol_types` checks their message types against `stem_protocol::kind` and `stem_protocol::decode::kind`; check the field names by hand (TD-16).
* A missing field of a stem worker message decodes silently as a default today (TD-16), so a renamed field fails nothing: check both sides by hand.  The decode messages (`stem_protocol::decode`) refuse a missing field.
* `stem-protocol` takes no workspace crate as a normal dependency, because both sides take it (LAYER-1, decision 0005).
* Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first four (MEM-1).  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm.
