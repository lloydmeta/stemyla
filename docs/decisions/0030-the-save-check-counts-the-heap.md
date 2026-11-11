# 0030: the memory check of `e2e save` counts the page's heap

Status: rejected (2026-11-11, by the maintainer: "writing a custom allocator is insane overengineering"; proposed on 2026-11-11 by the agent, in step 11 of M2 of `plans/active/tech-debt-rungs.md`)

The maintainer's answer: "I think you should simply measure and choose some kind of sane tolerance.  If it _really_ really makes no sense (prove why), then we can think again.  And avoid this kind of unjustifiable overengineering."  So D11 keeps the growth of the page's WebAssembly memory, and `WASM_BOUND_BYTES` is 3 MiB, from the spread measured in step 9, in CI run 43008561411 and in steps A and C (TD-97, closed).  The text below is the proposal as it was.

## Context

D11 of `xtask e2e save` checks MEM-4 for a save: a save of 10 minutes needs at most 1 MiB more of the page's memory than a save of 1 minute (`WASM_BOUND_BYTES`).  Until this decision, it measured the growth of the page's WebAssembly memory, `memory.buffer.byteLength` after the save less before it.  A WebAssembly memory grows in pages of 64 KiB and never shrinks ([MDN, `WebAssembly.Memory.prototype.grow()`](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Memory/grow)).  So the growth is the high-water mark after the save less the memory before it.

In Safari, this growth had two levels, about 1.9 MB apart, for each length and format (TD-97).  The check failed when the 1-minute save had the low level and the 10-minute save the high level: 1 run in 6 on the maintainer's Mac, and CI run 43008561411.  CI's `macos` job runs this check, and step 12 of M2 makes the job required.

Step A of `plans/active/tech-debt-rungs-m2-td97.md` measured 114 Safari saves and 30 Chrome saves with a sampler and a count of the Rust heap.  In each save, the page's Rust heap had the same live bytes at the save's start, and the same peak in the save, to 2.3 KB, for 1 and for 10 minutes.  The memory's growth still spread by up to 458,752 bytes.  So the growth depends on where the allocator finds free blocks when the save starts, not on what the save asks for.  No low save came in step A, so the layout of a low save is not known.  The Rust heap's count does not depend on the layout.

## Options

* Count the page's Rust heap.  A counting global allocator in `app`, on wasm32 only, wraps the default allocator.  At a save's start, the peak starts again from the live bytes.  At the offer, the page logs the peak above them.  The check compares those peaks.
* Compare the high-water marks after the two saves, not their growths.  This needs no change to the page.  It holds if a low save had more free memory at its start.  It fails if a low save fitted its buffers better.
* Read the memory before the save with the Save menu open.  This removes the growth of the menu's draw, about 1.9 MB.  But the save's growth then depends on whether the menu's buffer grew the memory, so the two levels can stay.
* Widen the bound.  TD-97 and step 11 of M2 forbid it: a wider bound hides a real growth with the length of the song.

## Decision

The check counts the page's Rust heap (`app/src/heap_count.rs`).  The counter is hand-written, with no new crate (DEPS-1).  It is generic over the inner allocator, so a native test checks it.  The page's global allocator is the counter over `std::alloc::System`, on wasm32 builds that are not test builds.  The native app keeps the default allocator.  `e2e save` reads the page's line `save: <name>: heap peak <peak> bytes above <start> bytes at the save's start, page wasm <bytes> bytes`.  It compares the peaks of the 10-minute and the 1-minute save against `WASM_BOUND_BYTES`, which stays 1 MiB.  The memory's growth stays in the run's log, not checked.

## Consequences

* The check no longer fails because of the allocator's layout.  A buffer in a save that grows with the song's length shows in the peak in full.
* The check does not see a growth of the memory that comes from the layout alone, with the same requests.  TD-97 keeps that gap.  In step A, such a growth did not grow with the song's length.
* The check measures from the save's start, after the click of the menu's item.  The short buffer of about 4 MB that the Save menu's draw makes is no longer in the measure.  The proposal recorded it in a new TD, as a cost of MEM-4.
* Each allocation of the page does two relaxed atomic operations more: an add and a maximum, or a subtraction.  The counter allocates nothing and waits for nothing (RT-1).
* A `realloc` that moves a block counts only its new size.  For a moment, the old and the new block are both held.  So the peak is low by at most the old size of a moved block.
* Only one save's measure is kept.  A save that starts while an earlier save waits for its encode starts the peak again.  Then the earlier save's line gives the peak since the later save's start.
