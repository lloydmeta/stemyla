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
* `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` and `xtask/src/e2e/` write and read messages by hand: change them in the same commit, and run `cargo run -p xtask -- e2e storage` (and `e2e decode` for the scripts in `xtask/src/e2e/`).  `e2e_messages_use_protocol_types` checks their message types against `stem_protocol::kind` and `stem_protocol::decode::kind`; check the field names by hand.
* A missing field of a stem worker message decodes silently as a default today (TD-16), so a renamed field fails nothing: check both sides by hand.  The decode messages (`stem_protocol::decode`) refuse a missing field.
* `stem-protocol` takes no workspace crate as a normal dependency, because both sides take it (LAYER-1, decision 0005).
* Clippy fails `slice::to_vec` in `app`, `stem-worker`, `decode-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first four (MEM-1).  A needed copy gets `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]` on its statement, field or match arm.
