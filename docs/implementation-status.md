# Version 0.2 implementation and qualification

October 3, 2026. This release provides an agent-testable local Linux SDR editing
service through a CLI and MCP stdio. The external agent owns conversation and
editorial decisions. It is a supported implementation slice of the
[product plan](product-plan.md), not completion of its M1–M5 camera, host and
advanced creator-workflow milestones.

## Implemented behavior

| Area | Current contract and evidence |
| --- | --- |
| Project authority | SQLite schema 4; immutable revisions and atomic head/history/request outcomes; exact rational source time; WAL/FULL sync |
| Editing and recovery | Validated atomic batches, dry-run, optimistic conflicts, same-key replay, request-outcome lookup, restore as a new revision, created/changed IDs, semantic diff |
| Agent interfaces | Shared strict typed JSON request API, CLI, MCP stdio; generated request schema; official MCP SDK integration; guide resource and supported operation examples |
| Resume and discovery | Compact resume; bounded history with through-revision pagination; bounded transcript search and job/artifact lists; capability/limit discovery |
| Ingest | Regular local files copied to content-addressed originals; staged import records and owner lock; owned abandoned staging recovery; original hashes checked before work; HDR/DOVI/stream-offset rejection |
| Source evidence | Metadata, bounded source frames, silence and scene evidence; cache keys include source/tool/parameters; cached frame hash validation; suggestions never edit automatically |
| Transcription | Optional pinned local whisper.cpp process adapter; model/executable/source fingerprints, bounded process and scratch checks; normalized source cues, cache hash validation and reuse; explicit attachment to project history |
| Creator workflow | Reviewed source transcripts; ordered source cuts into independent named outputs; 30 fps output mapping; imported font, measured caption wrapping within 80% canvas width, dark stroke, at most three lines; overflow refuses the transaction |
| Source protection | Exact source interval union in enabled, visible-opacity, normal-speed video tracks; respects solo; rejects lost coverage in edits/restores; explicit audited unprotect request |
| Jobs | Frozen revision/tool inputs; idempotent enqueue; per-project OS execution lock, heartbeat and generation fence; cancellation, interrupted-owner recovery and explicit retry with fresh attempts |
| Execution | Linux owned process groups, parent-death child termination, bounded stdout/stderr, deadlines and cancellation checks including file hashes; two render threads; duration/canvas/free-space checks |
| Export | H.264/yuv420p, constant frame cadence, square pixels, Rec.709 tags; full decode, planned frame/dimension checks and expected AAC duration; atomic final publish, manifest, indexed contact sheet and artifact hashes |
| Backup/migration | Consistent SQLite backup plus every historical original, hash validation, incomplete-copy guard, copied artifact jobs unavailable; schema 1–3 upgrades with pre-migration database backup |
| Distribution | Native 0.2 archive with checksums, no-root installer, licenses, dependency declarations, schema/agent guide and compiler/platform/commit manifest; tested runtime paths and MCP configuration |

The supported raw editing surface is project.rename, sequence.add/set,
track.add/set, clip.add, item.set/move/remove, text.add and caption.add.
Property discovery exposes the pinned domain registry. Rendering of every
possible property combination is not implied. Managed assets must enter
through import. Source protection certifies temporal coverage, not visibility
under another track, crop choices or editorial quality.

## Executed acceptance

The release qualification runs on Debian 13 x86_64, latest stable Rust 1.99.0,
FFmpeg n8.0.1 CPU/libx264 and DejaVu Sans. The application includes 21 passing
Rust tests with formatting and strict Clippy (`-D warnings`), covering atomic
rollback/concurrent writers, persistence/migration, queue ownership/retry,
exact protection, bounded subprocess cancellation, JSON/path conformance,
operation discovery, abandoned imports and caption overflow.

| Executed harness | Verified result | Practical limit |
| --- | --- | --- |
| application_smoke.py | Two-minute import; 30/31-second edit/revise/undo; 900/930 decoded frames; unchanged source, byte-identical undo, backup reopen/render, corrupt-source and protection rejection, upright rotation | Generated SDR fixture |
| job_smoke.py | SIGKILL worker, owned child terminates; next worker reconciles interrupted attempt; retry succeeds at attempt 2; cancellation finishes within five seconds and retains prior final | Linux process ownership and filesystem |
| agent_smoke.mjs | Official MCP SDK 1.32.0 handshake and tool schema; same CLI/MCP replay, key conflicts, cached frame, transcript/search, three independent captioned outputs, verified artifacts, explicit protection removal, scoped paths and backup | Real local MCP integration; no hosted-bot certification |
| media_matrix.py | Rotation 0/90/180/270 pixel comparison, synthetic HEVC SDR and VFR, video without audio; all outputs decode 60 frames; tagged PQ/HLG and nonzero starts rejected without committed assets | Does not prove real-camera color or speech lip sync |
| asr_smoke.py | Real tiny.en model on generated speech; recognized product/video; searchable source cues; analysis leaves history unchanged; cache reuse, wrong-model and altered-artifact/cache rejection | Machine text needs review; English fixture only |
| long_input.py | One-hour 256×144/10 fps source; five-second output at source 59:00; input seek at 3540 seconds; 150 decoded frames; render about 1.7 s and sampled process-tree peak about 145 MiB | Low-resolution CPU fixture, not 4K/iPhone performance or a hard memory reservation |
| Native bundle installation | Archive extraction and archive/binary SHA-256 validation; no-root installation, doctor with system-only PATH, official MCP workflow using the installed binary, including default 1080×1920 output | Same Debian 13 x86_64 host, not a separate machine qualification |

[Release evidence](../evaluation/service-results/qualification.json) records
summaries and provenance. Generated footage, models and project databases stay
outside Git. Historical candidate evaluation results remain separate.
CI executes Rust/schema plus application, worker, media-matrix and official MCP
checks and packages a native archive. Local ASR and the one-hour benchmark are
separate release checks. A checked-in CI workflow does not establish that a
remote Actions run passed; local execution is the evidence here.

## Remaining product scope

Real creator-supplied phone footage, phone appearance/audio review, a tested
HDR conversion path and an actual hosted-agent delivery trial remain open.
This build rejects unsupported HDR/offset paths clearly. There is no broad
claim of iPhone support, arbitrary languages or universal platform support.

Higher-level B-roll/audio narrative workflows, reusable versioned profiles,
selective source restoration helpers, named format/language variant operations,
timestamped review comments, workspace catalog, resumable remote transfer,
retention automation, published cross-platform installers and timeline
interchange remain in the [expanded backlog](implementation-backlog.md).
Projects can already contain multiple managed sources and independent outputs;
that does not certify all advanced composition or delivery combinations.

[Deployment](deployment.md) documents local access, worker supervision, storage,
upgrade/rollback and the supported limits. The generated service request schema
is the runtime envelope contract; older schemas/examples and proposed commands
in design specifications remain illustrative. Stable 1.0 acceptance requires
the broader documented camera/host and operational gates.

## October 3: HDR and timestamp conversion

Managed import now retains PQ/HLG originals and nonzero stream starts, preserves raw stream metadata, and reports inspectable versus deliverable media. Render plans apply a versioned linear-light Mobius tone map per HDR source before SDR compositing (100-nit reference white, 1,000-nit peak, BT.2020 to BT.709, limited-range dithering). Originals stay byte-identical. Compatible Dolby Vision profile 8 base layers are selectable; profile 5 has a precise unsupported delivery report. Generated 10-bit HEVC PQ/HLG, timestamp offsets, all four rotations, VFR and SDR HEVC pass full decode/frame-count and decoded reference pixel comparisons. Evidence: `evaluation/media_matrix.py`; camera appearance remains a human qualification gate. Contact sheets preserve aspect ratio and their hashes are verified on retrieval.

## October 3: Creator decisions and delivery

The typed `studio` API adds immutable profile/template versions, explicit profile
upgrades, independent format/language variants from frozen revisions, recorded
omissions with selective ripple restoration, visual-only B-roll replacement,
transcript corrections retained across reanalysis, artifact-anchored review and
source-based remapping. Raw edit discovery includes trim/split, effects, buses,
keyframes and transitions. Audio profiles use measured two-pass loudness and
peak QC. Sidechain ducking is implemented; its separate audible test is pending.
Every render includes SRT/VTT, a cover, aspect-preserving contact sheet and hashes.
Batch enqueue freezes all outputs atomically; independent jobs can be packaged
into a static local review bundle. Catalog, conservative cache collection,
backup manifests, restore and hash-based relinking are available. OpenTimelineIO
cut/track interchange has explicit loss reports for unsupported styling/effects.

`evaluation/studio_smoke.py` passes fresh-process selective restore, review remap,
correction/reanalysis, profile/template instantiation, identical dialogue samples
across B-roll replacement, measured loudness, vertical/square/Spanish-caption
landscape frozen exports, package hashes, backup restore, relinking and catalog.
The existing remaining-scope table above describes the prior 0.2 release and
will be replaced by the final qualification matrix after the integration checks.
