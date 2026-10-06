# Implementation and qualification

## Editorial planning on main, October 6, 2026

Current 0.4.0 source builds add read-only `plan-edit` and atomic
`apply-edit-plan`. A brief and individual cut reasons accompany source-preserving
candidates. Reports include exact frame-rounded source/output ranges, reviewed
cue excerpts, excluded source intervals including silent ones, complete scoped
proxy requests around both boundaries, and optional boundary expansion proposals.
Detected speech-boundary/handle and deliberate pacing exceptions require
specific per-cut notes before applying. The default speech handle is 250ms;
transcript cue timing is evidence, not guaranteed word-level alignment.

The plan SHA binds the complete plan and base snapshot. Applying commits its
brief, reasons, risk notes and content fingerprint with the sequence/history.
Preflight and frozen render manifests report current/stale/unplanned review;
source timing/policy/transcript changes invalidate review without deleting notes.
Unrelated project renaming retains current review. See [edit planning](edit-planning.md)
and the [additional editor research](editor-research.md#source-context-and-paper-edits-october-6-2026).

Local Rust 1.99 stable checks passed: **43 tests**, formatting, strict Clippy
and generated schema. The expanded official MCP SDK preservation trial passed
CLI/MCP planning and replay, atomic clipped-cue refusal, four verified source
context videos with audio, dry-run, frozen planned HDR rendering, transcript
review invalidation and backup retention. Media is generated PQ footage with
supplied burst-timing cues, not private human footage or artistic acceptance.
The planned-review HTML shows frozen output names, objectives and cut reasons.
Chromium playback/seeking, literal text escaping and layouts passed at desktop,
390px and 320px. Existing proxy/index/tracking/provider checks also passed.
The preservation regression still measured exact PQ/HLG lossless pixels,
640 luma values, retained static HDR metadata and the full-HD 393-frame edit
without a lossy intermediate. The [machine-readable record](../evaluation/service-results/edit-planning.json)
separates core-build regressions from final local sources and native CI evidence.

## Source-preserving 0.4.0 on main

October 5, 2026. Main supports source-matched plain edits with `assemble`,
read-only `edit-preflight`, 10-bit PQ/HLG HEVC masters and independently verified
SDR review previews. The published 0.3.1 archives below remain unchanged and do
not contain these features. Follow [natural edits](natural-edit.md) with a 0.4.0
source build. [Editor research](editor-research.md) records the official editor
documentation behind the workflow changes.

Local Linux x86_64 validation used Rust 1.99 stable and pinned FFmpeg 8.0.1.
All 36 Rust tests, formatting, strict Clippy and generated schema checks passed.
The final color-boundary regression also rejects generated tagged Display P3
SDR instead of relabeling it Rec.709; untagged SDR exposes its assumption.
The [machine-readable evidence](../evaluation/service-results/source-preservation.json)
records generated media separately from published/native CI evidence.

| Check | Observed behavior |
| --- | --- |
| `source_preserve_smoke.py` | Exact decoded PQ/HLG 10-bit YUV equality with lossless video, 640 distinct luma levels, retained HDR10 static metadata, 90 frames at 60fps and light/audio bursts at 0.5s/1.25s |
| High-quality encoding | Generated PQ ramp mean error 0.1381 in 10-bit code values at CRF 16; a controlled fixture, not arbitrary footage quality |
| Full-HD repeated cuts | 393 decoded 1080×1920 HLG frames from originals under the 4GiB child limit, without a lossy working conversion |
| SDR/preview integrity | Exact lossless SDR video, explicit PQ→SDR delivery, separate verified previews, corrupted preview rejection while the master remains available |
| Edit guardrails | Tight cuts, unrequested source reordering and mixed preservation contracts refuse atomically; intent overrides are durable; exact NTSC source starts |
| `combined_media_smoke.py` | Rotated PQ/VFR, nonzero container start and delayed audio stay aligned after explicit 30fps conformance; existing captioned mixed-color SDR checks pass |
| `preserve_agent_smoke.mjs` | Official SDK 1.32.0, CLI/MCP replay, dry-run, frozen HDR revision, verified master/preview retrieval, delivery and backup policy reopening |
| `browser_smoke.mjs` | SDR review playback, seeking and master/sidecar links at desktop/390px/320px Chromium viewports |
| Existing regressions | Application/history/backup/corruption, media geometry/color matrix and compositing patch comparison pass with the new wrapper |

[Native qualification](https://github.com/mingley/agent-video-workbench/actions/runs/37345429853)
passed at `717b5977fda03b501be89ae75598d4e0cba4a6bc`: 16 functional summaries
on x86_64 and 14 on ARM64, including color/memory, preservation and its SDK
trial. Both browser suites ran on x86_64; neither ran on ARM64. Downloaded
archives were checked against their checksums, binary hashes, source manifests
and root/third-party MIT notices. This is qualified CI evidence, not publication.
The subsequent SDR color-boundary guard adds the 36th Rust test. Its
[final native run](https://github.com/mingley/agent-video-workbench/actions/runs/37347469257)
passed at `88f9901a288609b823c4d54c20b7695f46837f9c`, again with 16 x86_64 and
14 ARM64 summaries. Both downloaded archives passed identity checks, and both
combined-media summaries explicitly record tagged Display P3 SDR refusal.

The documented baseline workflow also passed from a fresh project using the
installed 0.4.0 local bundle: 12 CLI calls, a ten-second 600-frame PQ master,
a separately verified SDR preview and delivery. The updated prepared MCP
configuration passed six official SDK calls; Chromium played and sought the
bundle at desktop and phone viewport sizes. These tests use generated footage
and retain the older demo separately. The current binary is locally built,
not an updated 0.3.1 public release.

Preservation is plain cuts only; HDR compositing, grades, captions and transitions
are not implemented. Supported sources are tagged limited-range Rec.709 or
10-bit BT.2020 PQ/HLG 4:2:0. Mixed dimensions/color/static metadata require
separate masters or qualified explicit SDR conversion. Lossless means video
encoding, not audio, file bytes or cadence/raster changes; audio is 48kHz stereo
AAC at 256kbps. Physical HDR display matching, real phone samples and Dolby Vision
dynamic metadata remain unqualified. No user footage was supplied to this run.

## Published 0.3.1 scope and retained evidence

October 3, 2026. This release provides a durable agent-operated Linux CLI/MCP
workbench. It closes the prior implementation gaps for HDR-to-SDR conversion,
creator workflows, transfer, analysis, maintenance, portability and delivery.
The external agent owns editorial decisions and file delivery. Claims below
are limited to executed tests; broad camera/host/appearance milestones still
require their own samples and accounts.

## Implemented contracts

| Area | Current behavior |
| --- | --- |
| Authority/history | SQLite schema 4, exact rational source coordinates, immutable revisions, atomic head/history/request outcomes, optimistic conflicts, dry-run, semantic diff and replay |
| Ingest | Owned content-addressed originals; staged copy/probe/hash/recovery; raw stream metadata; regular-file/hash checks; local or scoped HTTPS downloads |
| Transfer | Host/redirect scope, byte/space limits, strong-ETag range resume, changed-identity restart, optional SHA verification; transient signed URLs excluded from durable state |
| Color/geometry | PQ/HLG per-source linear-light Mobius tone map, BT.2020→BT.709, 100-nit reference/1000-nit peak, dithered limited range; SAR/orientation applied once; explicit RGB→BT.709 YUV conversion; nonzero starts and initial audio-gap padding |
| Source evidence | Bounded metadata/frame/silence/scenes, original-PTS frame indexes, mapped SDR proxies, hash-verified cache/attachments |
| ASR/corrections | Optional pinned whisper.cpp; model/program/source identities; reviewed transcript import/search; immutable analysis versions; corrections survive reanalysis; explicit reviewed selection and alignment checks |
| Creator edits | Independent named outputs, recorded omissions/selective ripple restoration, versioned profiles/templates, explicit upgrades, visual-only B-roll replacement, independent aspect/language variants |
| Captions | Source-linked cues, owned fonts, measured safe-area wrapping, reflow after corrections/style/translation, overflow/glyph refusal, burned text and matching SRT/VTT with style-loss report |
| Narrative/audio | Independent video/dialogue routes for J/L cuts, gain/EQ/compression/limiting/fades/buses, measured music sidechain ducking, optional measured two-pass loudness/peak QC |
| Framing/grade | Reversible basic SDR grade; selected-region CPU tracking with confidence/held fallback; accepted editable proposals render crop pans; linear/step position and constant viewport crop animation |
| Review/delivery | Exact-artifact feedback with source-point remapping and resolution state; atomic frozen batches; independent child outcomes; covers, indexed aspect-preserving sheets, sidecars and hash-verified HTML review bundle |
| Jobs/resources | Frozen revisions/tool inputs, persistent queue and priorities, OS owner lock, generation fence/heartbeat, bounded cancellation, interrupted-owner recovery/retry; child 4GiB address-space/32GiB file limits and bounded logs/deadlines |
| Lifecycle | Read-only project catalog, lease-aware cache planning/collection, opt-in collection after worker drain and schedulable workspace maintenance; historical originals and succeeded artifacts retained |
| Portability | Consistent verified backup manifest/restore, missing/corrupt verification and hash-based relink; portable profiles/templates/fonts; OTIO normal-speed cuts/gaps with exact-time extensions and explicit loss reports |
| Interfaces/releases | Shared strict generated CLI/MCP schema, scoped paths, compact resume and paginated discovery; official SDK integration; native x86_64/ARM64 archives, checksums, licenses and no-root installer |

Raw editing discovery covers trim/split/duplicate, tracks, effects, buses,
keyframes, transitions and markers as well as core clip/text/caption edits.
A domain property declaration is not a promise that every rendering combination
works: unsupported animation and track-effect paths fail precisely. Use item
effects or audio buses. Source protection certifies temporal coverage, not crop
visibility, overlay occlusion or editorial quality.

## Executed evidence

Latest stable Rust **1.99.0**, rustfmt and strict Clippy `-D warnings` pass;
**30 Rust tests**, zero ignored. FFmpeg is checksum-pinned CPU n8.0.1 with
libx264/libx265, zscale/tonemap and audio/caption filters. DejaVu Sans is the
qualified generated-fixture font; local ASR uses whisper.cpp v1.9.4 and tiny.en.

| Harness | Executed acceptance | Limit |
| --- | --- | --- |
| application_smoke.py | Two-minute ingest; 30/31s edits; 900/930 decoded frames; byte-identical undo, unchanged sources, rotation, protection/corruption refusal and backup reopen/render | Generated media |
| job_smoke.py | Actual SIGKILL stops owned child; interrupted recovery, successful attempt 2, cancellation within five seconds and prior final retained | Linux process/lock contract |
| media_matrix.py | PQ/HLG 10-bit HEVC tone mapping, rotations 0/90/180/270, normal/rotated SAR, HEVC SDR, VFR, missing audio, nonzero start, delayed audio; full decode and reference pixel comparisons | Generated HDR; no real-camera visual approval |
| studio_smoke.py | Selective restore retains another omission/later style, correction/reanalysis/selection, review remap, three sources, identical dialogue across B-roll replacement, loudness QC, pinned profiles/templates, vertical/square/Spanish landscape batch, verified review package, restore/relink/catalog/library and OTIO roundtrip | Supplied Latin-script translations; no translation model |
| audio_smoke.py | Frequency-measured music attenuation during speech, preserved dialogue and silence music, reversible mix and effect-based in/out fades | Generated tones; human speech A/B still required |
| timeline_smoke.py | Incoming/outgoing dialogue measured across J/L cuts, no duplicate embedded route, image/logo replacement retains decoded dialogue, crossfade handles and blended pixels | Synthetic narrative |
| transfer_smoke.py | Interrupted resume, changed ETag restart, checksum and scope rejection, tokenized URL absent from SQLite | Local development HTTP fixture; HTTPS required in normal configuration |
| analysis_smoke.py | PTS indexes, mapped proxy/cache, attachment tamper rejection, textured product tracking, occlusion hold, grade-plus-pan pixels, provider cache/validation/cancel without history changes | CPU template tracker; explicitly selected region |
| agent_smoke.mjs | Official SDK 1.32.0 CLI/MCP replay and errors, scoped paths, captions, four named outputs including full-HD default, profiles, frozen batch/review package, source index and portable reopen | Actual local stdio client, not every hosted product |
| asr_smoke.py | Real tiny.en recognition, searchable source cues, analysis leaves history unchanged, cache/model/artifact integrity checks | Generated English speech |
| long_input.py | One-hour 256×144/10fps source, five-second output from 59:00; source seek at 3540s; 150 decoded frames; about 1.1s render and 139MiB sampled process-tree RSS locally | Not one-hour 4K or a host memory reservation |
| highres_smoke.py | 3840×2160/60fps source to 1080×1920/30fps full decode, original unchanged; about 2.3s render locally | Two-second generated SDR source |
| Native CI/install | Actual Ubuntu x86_64 and ARM64 runners pass Rust/schema and media/creator/transfer/analysis/official-MCP checks; each installs its native archive without Cargo | Linux glibc route; no macOS/Windows claim |
| combined_media_smoke.py | Rotated offset HDR10/VFR with delayed audio; measured paired light/audio burst sync, mixed PQ/HLG/SDR captions; unsafe color/grade/HDR-output requests refuse | Generated camera-like media; no real phone approval |
| browser_smoke.mjs | Actual Chromium playback, seeking and sidecar links at 1280/390/320px; regression reproduces 0.3.0 viewport failure and verifies the 0.3.1 fix | Chromium emulation, not iOS Safari or a physical phone |

### Published 0.3.1

[0.3.1](https://github.com/mingley/agent-video-workbench/releases/tag/v0.3.1)
is published from `b1c17a1364225e9b1ae097c4afef7ff55c6923c9`.
[Native qualification](https://github.com/mingley/agent-video-workbench/actions/runs/37128053961)
passed 12 functional summaries on Ubuntu x86_64 and 11 on ARM64; browser testing
is explicitly skipped on ARM64. Both run Rust tests, formatting, strict Clippy
and schema checks. Local ASR and one-hour measurements remain separate.
[Publication](https://github.com/mingley/agent-video-workbench/actions/runs/37128475899)
verified the qualified archive/binary/source identities before publishing seven
assets. The public installer, expanded official MCP workflow and combined
HDR/sync checks passed against the downloaded x86_64 binary.

[Fresh virtual verification](virtual-verification.md) and its
[machine-readable record](../evaluation/service-results/virtual-verification.json)
separate each run's version and provenance. They include offline Ubuntu
installation, an independent MCP edit trial and the reproduced mobile review
failure fixed in 0.3.1. A subsequent
[infrastructure run](https://github.com/mingley/agent-video-workbench/actions/runs/37128475887)
passed both architectures with Node 24 Actions and zero workflow annotations;
it is separate from the frozen release source.

The [first-edit walkthrough](first-edit.md) was also executed with the published
0.3.1 binary on October 3: 19 successful CLI calls, two independent five-second
outputs with 150 decoded frames each, verified delivery, idempotent replay and
backup reopening at revision 5 with derived jobs unavailable. The same current
request files also passed 13 official MCP SDK calls in a separate fresh project,
including both verified renders and delivery. Its media was generated and
remains outside Git.

### Earlier baseline and research

[Baseline qualification JSON](../evaluation/service-results/qualification.json)
records the 0.3.0 feature-matrix summaries. That release's
[native CI](https://github.com/mingley/agent-video-workbench/actions/runs/37097054216)
passed ten harnesses on each architecture at
`7c520b50f44e1286563693180311c6030afed877`.
[Its publication](https://github.com/mingley/agent-video-workbench/actions/runs/37097667541)
verified identities before upload; public installation and MCP trials followed.
These older records are retained rather than relabeled as 0.3.1 measurements.

Historical candidate research is a separate comparison. Actions artifacts carry
source/platform manifests. Generated footage, model weights and project
databases stay outside Git. The prepared cloud demo retains its original media,
verified SDR delivery and independent variants; environment snapshot publication
is a separate product action.

## Explicit support boundaries

Published 0.3.1 delivery is **Rec.709 SDR H.264/AAC**, up to one hour per output and 4096 pixels
per canvas axis. Linux x86_64 and ARM64 with persistent local filesystem locking
are qualified. Mac/Windows binaries, arbitrary consumer-agent accounts and
phone playback/appearance are not certified by these Linux tests.

Dolby Vision profile 8 with HDR10/HLG compatible base layer is recognized;
actual Dolby Vision footage is not in the generated qualification matrix.
Profile 5, unqualified BT.2020 non-PQ/HLG or Log transforms, ProRes/Log/Cinematic
metadata, HDR-output masters and GPU encoders need additional implementation
and sample qualification in that older release. The 0.4.0 plain-cut preservation
scope is documented above; HDR composition and advanced color work remain separate.

Basic grade covers brightness/exposure/contrast/saturation; other color parameters
and unsupported keyframes refuse delivery. Geometry pans require linear/step,
normal-speed footage, constant viewport and no simultaneous transition.
Tracking is an optional selected-region template matcher with visible uncertainty.
Language variants require explicit translations; complex shaping/RTL and arbitrary
scripts/fonts need dedicated fixtures. OTIO covers normal-speed clips/tracks/gaps,
reports omitted captions/effects/transitions, and requires SHA bindings for import.

The [backlog](implementation-backlog.md) records these qualification/extensions
without retaining stale claims that implemented 0.3 features are still missing.
[Deployment](deployment.md) describes workers, storage, maintenance and rollback.
