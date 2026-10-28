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

The third option.  `app::memory_path_frames(memory_limit, lanes)` gives the limit, with `app::LANE_BUDGET` = 7.  `lanes` counts the lanes that the page holds in memory after a separation on the memory path.  These are the sources in memory and the input if it is not one of them.  They are also each other derived lane in memory (classic drums) and the 6 stems.  The page holds the old song, with its classic drums lane, until a separation ends.  Classic drums that reads stored lanes first counts its own lanes (`app::classic_lanes`).  These are each lane in memory of any kind, each source that it reads in, and its new lane.  Both counts use one function, `app::held_lanes` (finding 4-1 of review round 4).  Every check of the memory path's length and its status line use the function.  These are the opens and restores that decode on the page, and the separation's path.  They are also the memory path after storage refuses the mix or the stems, and classic drums that read stored lanes first.  The last is the read of a stem that is not complete.

An open that decodes on the page reads each file's head first.  It refuses a WAV or a FLAC whose header states a length past the limit.  It also refuses a file larger than the largest file of its stated length, or of the limit if it states none (`engine::decode::largest_file`).  That file has two channels of 32-bit samples at 384 kHz for a WAV or a FLAC, or 320 kbit/s for an MP3.  It also has 16 MiB for tags.  A restore takes the length from the session's `meta` when it has one, and compares each file's size with the largest file of that length.  Then the page reads, decodes and drops one file at a time (`engine::song::SongWithin`), and the decode stops at the limit.

## Consequences

* With 7 lanes, one source gets 100 s and two sources 77 s.  Six sources (13 lanes) get 53 s, and 21 sources (28 lanes) 25 s.  The read-ahead buffer sends most songs of more than about 20 sources to the page (D2 of the M7 plan).  These are refused unless they are short.
* A song that was separated in memory and is separated again holds its 6 old stems too, so it gets 53 s.
* One source with a classic drums lane in memory holds 8 lanes, so it gets about 87.5 s.
* The status line of a refusal gives the length, the limit for the song's lanes and why the song is on the memory path.  It rounds the length up and the limit down to whole seconds (finding 4-2 of review round 4).  It also says what to do: free browser storage when storage failed, else open a shorter song.
* A mono source counts as a full lane.  Its two sides share one buffer, so the count is high for it.
* A file is read whole only up to its cap.  A WAV or a FLAC of more than two channels at a high rate can be refused although it fits.  A short file with more than 16 MiB of tags can also be refused.
* The decode on the page converts each source in blocks to 44.1 kHz, so it holds no samples at the source's rate.  It holds the bytes of one file at a time, while that file decodes (finding 1-3 of review round 1 of `plans/active/stream-from-storage-catchup.md`).  It reads a file into wasm memory in blocks (`app::picker::read_blob`).  Each read asks JavaScript for never more than one block of 1 MiB (finding 2-3 of review round 2).  Each block is garbage after its copy.  So the tab holds at most the lanes and the bytes of one file, plus the blocks that the garbage collector has not freed.
* M10 of `plans/active/stream-from-storage.md` measures the memory path, and can change `MEMORY_PATH_MAX_FRAMES` or `LANE_BUDGET` on its measures.
