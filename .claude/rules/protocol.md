---
paths:
  - "protocol/**"
  - "worker/src/web.rs"
  - "app/src/separator.rs"
  - "xtask/src/e2e.rs"
  - "xtask/e2e/**"
---

# The page <-> worker wire format

LAYER-1, PROTO-1 and PROTO-2 in `docs/invariants.md`.  The format is documented at the top of `protocol/src/lib.rs`.

* Change a message in `stem-protocol` only, with its encoder and decoder together, and extend `protocol/tests/web.rs` (round trip through a structured clone with its transfers).
* `xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` write and read messages by hand: change them in the same commit, and run `cargo run -p xtask -- e2e storage`.  `e2e_messages_use_protocol_types` checks their message types against `stem_protocol::kind`; check the field names by hand.
* A missing field decodes silently as a default today (TD-16), so a renamed field fails nothing: check both sides by hand.
* `stem-protocol` takes no workspace crate as a normal dependency, because both sides take it (LAYER-1, decision 0005).
