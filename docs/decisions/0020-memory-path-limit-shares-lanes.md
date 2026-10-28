# 0020: the memory path's limit shrinks with the lanes in memory

Status: proposed (2026-10-28, by the agent)

## Context

Decision 0015 sends a song to the memory path when browser storage is unavailable or full, and says that the memory path refuses a song longer than fits.  Until now, one limit stood for every song: 100 s at 44.1 kHz (`app::MEMORY_PATH_MAX_FRAMES`).  It came from a separation of one source: the page then holds the source and the six stems that `app::assemble` makes, 7 stereo lanes of `f32`.  At 100 s, one lane is 35.3 MB, so 7 lanes are about 247 MB.

The limit did not count the lanes.  A song of 21 sources holds 21 source lanes, the mix of them that the separation takes, and 6 stems: 28 lanes, about 988 MB at 100 s.  An open that decoded a song on the page had no limit at all but `engine::decode::MAX_DECODED_BYTES` (decision 0013), 1 GiB for each file (TD-58).  An open decodes on the page when there is no session, the decode worker cannot keep the session's files, or the song's lanes do not fit the read-ahead buffer (D2 of `plans/active/stream-from-storage-m7.md`).  iOS stops a tab at 2 GB (MEM-4).

## Options

* One limit in frames for every song, as before.  A song of many sources passes the memory that the limit protects, up to 4 times for 28 lanes.
* A limit in bytes of decoded audio on the page.  It is exact, but the status line cannot give it as a length, and every check must count bytes at the source's rate.
* A limit in frames that shrinks with the lanes that the page holds in memory: `min(limit, 7 x limit / lanes)`.  The page never holds more than 7 lanes of the limit's length, which is what the old limit allowed for one source.

## Decision

The third option.  `app::memory_path_frames(memory_limit, lanes)` gives the limit, with `app::LANE_BUDGET` = 7.  `lanes` counts what the page holds in memory after a separation on the memory path: the sources in memory, the input if it is not one of them, and the 6 stems.  Every check of the memory path's length and its status line use it: the opens and restores that decode on the page, the separation's path, the memory path after storage refuses the mix or the stems, classic drums that read stored lanes first, and the read of a stem that is not complete.

An open that decodes on the page reads each file's head first.  It refuses a WAV or a FLAC whose header states a length past the limit, and a file that states no length and is larger than the largest file of a song within the limit (two channels of 32-bit samples at 384 kHz for a WAV or a FLAC, 320 kbit/s for an MP3, and 16 MiB for tags).  A restore takes the length from the session's `meta` when it has one.  Then the decode stops at the limit (`engine::song::Song::load_files_within`).

## Consequences

* With 7 lanes, one source gets 100 s, two sources 77 s, six sources (13 lanes) 53 s, and 21 sources (28 lanes) 25 s.  Most songs of more than about 20 sources, which the read-ahead buffer sends to the page (D2 of the M7 plan), are refused unless they are short.
* A song that was separated in memory and is separated again holds its 6 old stems too, so it gets 53 s.
* The status line of a refusal gives the length, the limit for the song's lanes, why the song is on the memory path, and what to do: free browser storage, or open a shorter song.
* A mono source counts as a full lane.  Its two sides share one buffer, so the count is high for it.
* A file whose head states no length is read whole only up to the cap.  A WAV or a FLAC of more than two channels with no stated length can be refused although it fits.
* The decode on the page converts each source in blocks to 44.1 kHz, so it holds no samples at the source's rate.  It still holds each file's bytes while that file decodes.
* M10 of `plans/active/stream-from-storage.md` measures the memory path, and can change `MEMORY_PATH_MAX_FRAMES` or `LANE_BUDGET` on its measures.
