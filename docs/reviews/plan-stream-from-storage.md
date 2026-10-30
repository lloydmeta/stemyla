# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - |
| 2026-10-25 | 1 | 4e076dd | 1-1 | reviewer | blocker | bug | MEM-4 | plan M5, Purpose | No milestone stops reading the whole encoded source into memory | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-2 | reviewer, docs-reviewer | should-fix | doc | DOC-1 | plan :56, 0017 :7 | cpal queues about 93 ms, not 186 ms; M1's 100 ms block exceeds it | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-3 | reviewer | should-fix | bug | MEM-4 | plan M7 | Classic drums, MixFollower mid and lane_signal make song-sized buffers | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-4 | reviewer | should-fix | rule | MEM-5 | plan M1, M3, M4, M5, M8 | New OPFS writes cite no MEM-5 and have no spy test | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-5 | reviewer | should-fix | doc | VERIFY-1 | plan M9 | M9 closes TD-11 without an absolute budget | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-6 | reviewer | should-fix | doc | none | 0015 :23 | Storage cost omits the mix and source files | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-7 | docs-reviewer | should-fix | doc | none | plan :152, 0016 | Older sessions already have stem WAVs; restore not stated | confirmed | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-8 | reviewer, docs-reviewer | nit | doc | none | plan Memory budget | Fixed parts not derived; load_song mix not counted | unvalidated | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-9 | docs-reviewer | nit | doc | none | plan :5 | Purpose omits overviews | unvalidated | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-10 | reviewer, docs-reviewer | nit | doc | none | plan :159 | M0 not ticked | unvalidated | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-11 | docs-reviewer | nit | rule | none | plan (8 lines) | Sentences over 25 words | unvalidated | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 1 | 4e076dd | 1-12 | reviewer | nit | doc | none | plan M1 | M1 measures at tempo 1 only, not MAX_TEMPO | unvalidated | none | 0125a5b review round 1 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-1 | reviewer, docs-reviewer | should-fix | doc | GPU-1 | 0018 :12, plan M9 | naga 30.0.1 needs SHADER_FLOAT16_IN_FLOAT32 for unpack2x16float | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-2 | docs-reviewer | should-fix | doc | none | plan M9, 0018 | Matmul kernels also run activation products; f16 needs variants | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-3 | docs-reviewer | should-fix | doc | DOC-1 | 0015 :7 | 190 MB per minute but the listed parts add to 170 | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-4 | reviewer | should-fix | doc | VERIFY-1 | plan :186, :189 | Storage fallback decided but not built or tested | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-5 | reviewer | should-fix | weak-test | VERIFY-1 | plan M10 | 32 MB growth tolerance allows about 300 MB at 30 minutes | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-6 | reviewer | should-fix | doc | none | plan M1, 0016 | Spike and read rates leave out the f32 original lane | confirmed | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-7 | docs-reviewer | nit | doc | none | plan :72 | Band-split gamma is applied on the CPU | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-8 | reviewer, docs-reviewer | nit | doc | none | plan Memory budget | load_song mix is freed; 1720 MB is an upper bound | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-9 | reviewer, docs-reviewer | nit | doc | none | plan :54-55 | Overviews have a fixed size | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-10 | docs-reviewer | nit | doc | DOC-1 | 0015 :11, plan :5 | Stale 6-minute and 2-minute figures | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-11 | reviewer | nit | doc | none | 0017 :17, plan M1, M6 | Seek plays old audio, but M1 and M6 expect silence | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 2 | 3136e1f | 2-12 | reviewer | nit | doc | LAYER-1 | plan M3 | Worker's WAV header writer and size patch not named | unvalidated | none | cc9cad7 review round 2 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-1 | docs-reviewer, reviewer | should-fix | doc | DOC-1 | 0017 :21, 0016 :7, plan :57 | Read-ahead buffer counts seven lanes, not eight | confirmed | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-2 | reviewer | should-fix | other | VERIFY-1 | plan M5, M10 | Decode worker memory is in no check or budget | confirmed | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-3 | reviewer | should-fix | doc | none | 0018 :28 | Amended accepted record has no maintainer acceptance | confirmed | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-4 | reviewer | should-fix | other | none | plan :7 | 30-minute separation takes 49-85 min; no wake or device conditions | confirmed | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-5 | docs-reviewer | nit | doc | none | 0018 :28 | naga 26.0.0 is not eframe's | unvalidated | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-6 | docs-reviewer | nit | doc | none | plan M9 | Shader-list wording misstates the GPU rule | unvalidated | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-7 | reviewer | nit | doc | none | plan :5, TD-54 | 1.7 minutes is a bound; TD-54 heading says 2 | unvalidated | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-8 | reviewer | nit | doc | none | plan :18, :38 | 48 chunks is at most 13.5 M frames | unvalidated | none | b2b5260 review round 3 fixes |
| 2026-10-25 | 3 | cc9cad7 | 3-9 | reviewer | nit | doc | none | plan M3 | Fallback limit leaves out unmeasured parts | unvalidated | none | b2b5260 review round 3 fixes |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - | - | - | - |
| 1 | 9 | 298910 | 6.8 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0; no mutants in the diff | - | - | - | - | - | - |
| 2 | 8 | 340268 | 10.5 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0; no mutants in the diff | - | - | - | - | - | - |
| 3 | 6 | 264395 | 8.7 | 0 | 1 | 0 (auto mode; a fallback prompt is not counted) | mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0; no mutants in the diff | - | - | - | - | - | - |
