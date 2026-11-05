# Implementation and qualification

## Source-view latency improvements on 0.5.1 source

The runner wakes on child exit instead of waiting for a polling interval.
Storyboards compose up to four accurately sought inputs per row; coded frames
larger than 1920×1080 retain sequential decoding. Preview output timestamps and
full audio/video validation share one decode. Byte-budget errors report the
smallest measured encoding attempt. Original integrity checks, request types,
duration/byte caps and native frame mappings retain their semantics.

Five interleaved generated 120-second 720p/30fps runs measured median storyboard
time of 2.312s → 0.902s and ten-second/300-frame preview time of 2.315s → 1.940s.
Cached recipes fell from about 0.19s to 0.10s. Decoded pixels/audio/timestamps
match for every measured recipe. Storyboard peak single-process memory rose
from about 33MiB to 105MiB on that fixture. The local suite passes 183 Rust tests,
strict lint/format/schema/build checks, extended source-view and official MCP
checks, application exports and worker recovery/cancellation. Native ARM64 and
release publication require their separate qualification. See
[research and scope](source-view-performance.md) and
[the reproducible evidence](../evaluation/service-results/source-view-performance.json).

## Source browsing and constrained review media (0.5.0)

`source-storyboard` builds cached pages of uniformly sampled source frames with
observed PTS/time bases, rational source positions, tile coordinates, sparse
coverage limits and bounded active-transcript context. A reported audio-only
tail does not create phantom video samples. `budget-preview` measures actual
output duration and bytes, fully decodes the artifact, preserves source frame
order including VFR, and pairs original/proxy PTS. Unsatisfied budgets or
ambiguous mappings fail without a successful artifact. Up to three disclosed
resolution/CRF attempts fit byte caps; no temporal downsampling or silent trim
is used. Source originals and editing history remain authoritative.

Local Linux x86_64 checks passed 180 Rust tests, strict Clippy, formatting, the
release build, generated-schema and six publisher-gate tests. Generated media
exercised pagination, cache reuse/invalidation, frozen transcript context,
actual timestamps, B-frame boundaries, nonzero source origin, VFR, a measured
250ms audio/light offset, HDR-to-SDR conversion, rotation, impossible size and
actual-duration refusals, and source/attachment tamper rejection. Official MCP
SDK checks exercised PNG delivery, preview jobs, CLI replay equivalence and WAV
excerpts. Native qualification/publication uses 36 x86_64 and 31 ARM64 summaries,
including these new workflows; exact source/run records are bound by the release
request and published native manifests. Local results alone are not ARM64 proof.

[Native qualification](https://github.com/mingley/agent-video-workbench/actions/runs/42055909251)
passed on source commit `d025611bd3db56209c03c5d70d2e472ffc600809` for both
architectures: 180 Rust tests passed on each, with two existing opt-in tests
ignored by the ordinary suite. All 36 x86_64 and 31 ARM64 functional summaries
passed, including installed CLI/MCP workflows and x86_64 browser checks.
The [0.5.0 release](https://github.com/mingley/agent-video-workbench/releases/tag/v0.5.0)
publishes those exact archives. Its public installer verified the x86_64 binary
against the native manifest; fresh generated source-view and official MCP SDK
checks passed against that downloaded binary. ARM64 was tested in native CI,
not installed on the local x86_64 host. See the
[release evidence](../evaluation/service-results/release-0.5.0.json) for identities,
checksums and limits.

On the generated 120-second 720p/30fps source, three uncached/warm recipe runs
measured the following medians, including queue/worker/integrity/artifact lookup
and excluding initial source generation/import. See [the reproducible record](../evaluation/service-results/source-views.json)
and `evaluation/source_views_benchmark.py` for exact samples and hardware.

| Operation | Uncached recipe | Cached recipe |
| --- | ---: | ---: |
| Twelve-frame source storyboard | 2.62s | 0.21s |
| Ten-second, 300-frame bounded preview | 2.52s | 0.21s |

The filesystem was warm, on an AMD EPYC 9V74 host with a four-CPU quota and
16GiB container limit. These synthetic CPU figures do not establish real-camera,
4K, model/ASR, hosted agent or physical-device performance. Sparse samples can
miss brief events; model-dependent content understanding remains external.
See [source views](source-views.md) for API usage, budgets, mappings and limits.


## Export verification and first-edit feedback

Current source shares the verification decode with contact-sheet generation,
reports stage timings, rejects invalid imports before staging, and fixes sampled
frame metadata for very short exports. It also retains the earlier CLI result-field
extraction, new-project creator metadata and clarified setup/defaults.
[The measured report](performance-feedback.md) records the before/after timings,
identical decoded pixels/audio, longer-than-ten-second reproduction, local tests
and remaining platform/media limitations. These changes are included in 0.5.0.

## Creator-agent toolkit on current source

The current source adds bounded source-evidence pages, proposals for highlight
plans, and a source-bound contract for an explicitly configured moment-search
provider. Agents can receive verified frame, contact-sheet and short audio
content over MCP or its CLI/JSON equivalent. Word alignment, translation and
subject observations are supplied by an agent or provider, validated against
the source or captions, and kept reviewable; the workbench does not include a
semantic model, aligner, translator or subject detector. Crop suggestions
refuse existing geometry and effects that invalidate their sampled fit check.

Offline review pages support side-by-side playback, timestamped notes and an
explicit render-bound decision. Import verifies that artifact, commits the
draft atomically and supports replay. Checked batch packages retain individual
pass/fail/unknown results and successful outputs; they omit full project
snapshots, saved reviews and edit notes. Selected media, captions, images,
identities and caller-authored metadata remain visible and need review before
sharing. The older `delivery` package retains full project snapshots and may
include saved reviews. Thumbnail candidates use creator-selected times from a
verified SDR review. Optional per-item spectral noise reduction applies after
gain, pan and fades; a generated tone fixture measured approximately 20 ms of
added onset delay, so listen and check synchronization.

On local Linux x86_64, 166 Rust tests, formatting, strict Clippy, the release
build, generated-schema check and all 35 required native-workflow summaries
passed. The generated-media and official MCP SDK checks exercise these workflows,
including a recursive privacy regression for a real batch package and feedback
import against its minimized review page. Chromium tests review decisions and
side-by-side playback at desktop and phone-width viewports. Managed inspection
and analysis paths now reject existing symlinks, and the project database is
opened with SQLite's no-follow flag. These checks do not provide filesystem
isolation against concurrent component replacement, hard links, bind mounts or
SQLite sidecars. The tests do not establish real-creator acceptance, physical
phone or HDR appearance, translation or speech accuracy, or model consumption
of media in a particular host. See the [creator-agent toolkit](creator-agent-toolkit.md).

[Native qualification](https://github.com/mingley/agent-video-workbench/actions/runs/41579096448)
passed for source commit `2ce17fbcc98b92c1ec9199af47bbda5708ae03f6` on Linux
x86_64 and ARM64, with 35 and 30 required workflow summaries respectively;
browser tests run on x86_64. The [0.4.0 release](https://github.com/mingley/agent-video-workbench/releases/tag/v0.4.0)
contains the two qualified archives. The public installer verified and installed
the x86_64 release locally; its binary hash matched the published native
manifest, and the official MCP SDK media, host-readiness and checked-batch
workflows passed against it. ARM64 was qualified in native CI, not installed on
the local x86_64 host.

Earlier sections retain the evidence and feature scope reported at the time of
those checks; later source changes can extend their behavior.

## Earlier research-backed creator delivery validation

`transcript-search` now offers opt-in canonical Unicode normalization and
no-space cue joining while retaining its original defaults. Normalization and
lowercasing apply per cue; split combining sequences, language-aware word
boundaries and full case folding are not established. Search remains literal,
with whole-cue timing and source listening still required.

`delivery-check` evaluates one user-supplied brief against a completed queued
render. It rehashes the master and freshly probes it for metadata-dependent
checks, returning pass, fail or unknown for requested geometry, aspect,
container and stream duration, bytes,
sidecars and color tags. Sidecar checks reconstruct frozen caption data; they
cannot establish accuracy or completeness. `chapter-draft` formats explicitly
selected frozen markers with caller-supplied titles, exact marker times and
rounding disclosures. It reports documented timestamp checks while leaving
account/video eligibility unknown. Both refuse synchronous jobs without a
sequence-bound queue record. The render manifest is not independently
authenticated; reports do not establish platform acceptance, visual quality or
editorial approval. Delivery reports are returned for individual renders and
are not yet embedded in packages or evaluated as a batch. See [delivery checks
and chapters](creator-delivery-checks.md) and the [research](creator-delivery-research.md).

Visible caption sidecars now use exact millisecond rounding and refuse cues that
would collapse to zero duration before starting a new encode. SRT consumers can
interpret literal markup differently. Review ranges use recorded verified
master/preview timing, including explicit sequence duration. If timing evidence
is missing, new range submissions are refused and a newly generated review page
keeps playback and point references while explaining why range controls are
unavailable. Existing artifacts, review pages and previously saved request
outcomes remain unchanged; regenerate the package or render as needed.

Local Linux x86_64 validation used Rust 1.99.1 and FFmpeg 8.0.1. All 99 Rust
tests, formatting, strict Clippy, release build and generated-schema checks
passed. Generated-media tests exercised search, chapter and delivery checks
through named CLI, JSON and the official MCP SDK, including pass/fail/unknown,
unchanged originals and master tamper rejection. Both queued and synchronous
caption renders refused a sub-millisecond cue before invoking the backend, with
prior artifacts intact; a separate queued captioned render passed the sidecar
brief, and altered SRT bytes failed it. An explicit six-second render with four
seconds of content accepted feedback in its last two seconds; missing evidence
was refused without losing a prior replay. Chromium checked fallback and range controls at
desktop and phone-width viewports, including clipboard fallback. Existing
comparison and creator workflow regressions also passed. These tests use
generated media and viewport emulation; real creators, physical devices,
account-specific platform behavior and a new ARM64 build remain unqualified.

## Frozen comparison and source-review improvements on current source

`compare-outputs` compares source intervals from two successful queued
source-preserving plain-cut renders. It identifies retained, added, omitted,
repeated and uniquely reordered footage, with exact source/output coordinates.
Layered or styled compositions and older synchronous renders without a
job-bound sequence are reported as unsupported. Source-range agreement is not
an audio, color, cadence, appearance or artistic comparison; the render manifest
itself is not independently authenticated. See [compare edits](compare-edits.md).

Delivered review pages also allow a creator to mark and copy an output-time
range. Captioned composition refuses a nonempty cue intersection that would
round to zero frames instead of silently dropping its text; a previously saved
outcome still replays unchanged. Source inspection bounds its detectors to the
requested range and reports uncertain timestamp and stream-coverage information.
Existing rendered artifacts do not change: review affected earlier captions and
create a new output and render when a correction is needed.

Local Linux x86_64 checks passed with Rust 1.99.1 and FFmpeg 8.0.1: 76 Rust
tests, formatting, strict Clippy, the release build and generated schema. A
generated-video official MCP/CLI comparison exercised frozen revisions,
reordering, additions, omissions and tamper rejection. Generated inspection
fixtures covered delayed and discontinuous audio, nonzero container origins,
transport-stream seeking, coarse clocks, low sample rates and attached artwork.
A caption regression checked both sidecars and burned one-frame text; the
creator workflow and Chromium review controls passed with three generated
outputs at desktop and phone-width viewports. These results do not qualify
real cameras, physical devices or a new ARM64 build.

## Creator workflow and rendering reliability on current 0.4.0 source

The source build adds opt-in [cross-cue transcript search](transcript-search.md),
separate transcript excerpts for each paper-edit boundary, and
[source-range feedback mapping](review-feedback.md) for normal-speed video and
audio. Feedback reports missing, repeated and rearranged material and retains
legacy point-based records. A review page can copy the frozen render ID,
revision and playback time for a creator to share with their agent.

Analysis requests now validate before queueing; generated preview requests bind
their project, source and media configuration. Cached source inspection returns
the requested asset ID even when two assets share the same original bytes.
Disabled solo tracks no longer suppress enabled tracks during rendering, and
rendering rechecks protections so older unsafe snapshots cannot silently hide
protected footage. Enabled visual solos also participate in protection checks.
Subtitle sidecars now follow visual solos independently of audio solos.
Existing render artifacts and sidecars are unchanged; review and re-render any
affected earlier output to regenerate them.

Local Linux x86_64 validation used Rust 1.99.1 and FFmpeg 8.0.1. All 62 Rust
tests, formatting, strict Clippy, release build and generated-schema checks
passed. Generated-media regressions verified protected picture and audio under
disabled solos, explicit refusal of unsafe older snapshots, independent enabled
solos, analysis and creator workflows, and multi-cut feedback after removal.
The official MCP SDK workflow found a phrase across cues and verified its audio
proxy, replayed the request, and exercised paper edits and frozen review. Chromium
played and sought the delivered review at 1280, 390 and 320 pixels, including
copied references and clipboard fallback. These are generated fixtures and
viewport emulation, not human or physical-device qualification.

These changes do not establish which image layer or screen region a review
comment refers to, semantic transcript search, real-creator approval, physical
phone appearance, or additional operating-system support. The
[creator opportunities](creator-agent-opportunities.md) plan lists further work
and qualification gates. These additions have not been qualified on a new
native ARM64 run.

## Review freshness on current 0.4.0 source

Edit-plan review now includes the output's protected source ranges in its
freshness check and reports the reason when review is stale. Protections on
other outputs do not invalidate it. Records made before this check are compared
with the empty protection set they had when their new output was created; their
notes and history remain intact. Render manifests retain the state frozen with
their revision.

On local Linux x86_64, Rust 1.99.1 passed 44 tests, formatting, strict Clippy,
the release build and generated schema check. The official MCP SDK workflow
passed with a generated 60fps PQ fixture and FFmpeg 8.0.1: CLI/MCP agreed on
protection-related staleness, an unrelated output stayed current, removing the
new protection restored the reviewed state, and the already-frozen render kept
its original review state. The earlier [native run](https://github.com/mingley/agent-video-workbench/actions/runs/41098553110)
passed on x86_64 and ARM64 at commit `6517e40`; the subsequent additions above
are not covered by that run.

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

The installed `196286f` source bundle also passed the documented planning workflow
against the prepared cloud demo: ten CLI calls (including named aliases) and ten
official MCP SDK calls. The retained baseline and new candidate render 600 and
540 verified PQ frames at 60fps respectively. Their combined review page passed
Chromium playback/seeking and note display at all three viewports. The original
generated source is unchanged, and project history is retained at revision 3.

[Final native qualification](https://github.com/mingley/agent-video-workbench/actions/runs/37547998545)
passed at `196286fd911992d50deb08cf0e13e8324e01f28b`: 43 Rust tests and
16 functional summaries on x86_64 / 14 on ARM64. Both native SDK summaries
record the new planning and context-audio checks; the planned-note browser
checks run on x86_64 only. Both downloaded archives passed source, binary and
archive checksum verification, with root and third-party licenses matching the
source exactly. This qualifies the source build; it does not publish a release.

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
