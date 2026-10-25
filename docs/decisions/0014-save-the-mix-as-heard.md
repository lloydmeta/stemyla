# 0014: the mix lane saves the mix as heard, mixed on the page

Status: accepted (2026-10-25, by the maintainer)

## Context

Users ask to save one file of some stems, such as every stem except the vocals.  Each stem lane has a "Save" menu (WAV, FLAC, MP3).  The mix lane has none.  The player already defines what the user hears: `SharedParams::effective_gain` applies Mute, Solo and volume to each stem.

Prior art: Ableton Live's export of the Main track "will sound exactly like what you hear through the Main output" ([manual, 5.1.3](https://www.ableton.com/en/manual/managing-files-and-sets/)).  Moises exports an "Audio Mix" with the user's changes ([help](https://help.moises.ai/hc/en-us/articles/10554030191004-How-do-I-export-my-song-with-the-changes-made)).

## Options

The UI (mockups on 2026-10-25):

* A: a "Save mix..." dialog with a tick for each stem.  It is a second selection beside Mute and Solo.
* B: a toolbar button that saves the stems that Mute and Solo let through.  Nothing shows what is in the file.
* C: a "Save" menu on the mix lane, for the mix as heard.  Its subtitle names the stems in it.
* D: an "in mix" tick on each lane.  A third toggle beside Mute and Solo.
* E: "the song without <stem>" in each stem's menu.  One subset only.
* F: C, with ticks that set Mute.

Where to mix:

* On the page, in blocks, into JS arrays.  The worker encodes FLAC and MP3 as it does today.
* In the worker.  The page must copy every stem to JS (six times the data), and the worker needs `dsp` or `engine` (LAYER-1).

## Decision

Option C.  The mix lane's "Save" menu saves the mix as heard: each stem at `effective_gain`, summed, clamped to ±1, at 100% tempo.  The page mixes in blocks over several UI frames, into JS arrays.  The worker and the wire format do not change.

## Consequences

* Mute and Solo are the selection.  The user hears the file before saving it.
* The subtitle of the mix lane and the file name say which stems are in the mix.
* The page's WebAssembly memory holds only blocks, not the mix (MEM-1, MEM-4).
* The main thread shares its time with the Web Audio callbacks.  So each UI frame mixes for a few ms only.
* The file does not follow the tempo control.  The menu says so.
* An audible "original" lane is in the mix too, so the user can save a doubled mix.
