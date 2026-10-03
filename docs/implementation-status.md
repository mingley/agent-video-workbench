# Version 0.3 implementation and qualification

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

[Qualification JSON](../evaluation/service-results/qualification.json) records
current summaries and provenance. Actions artifacts carry their own source
commit/platform manifests; successful workflow execution is observed before
release publication. Local ASR and one-hour measurements are separate checks.
Generated footage, model weights and project databases stay outside Git.
Historical candidate research evidence is separate from this release.

[0.3.0 is published](https://github.com/mingley/agent-video-workbench/releases/tag/v0.3.0)
with native archives, combined checksums and dependency setup helpers.
[Final native CI](https://github.com/mingley/agent-video-workbench/actions/runs/37097054216)
passed all ten workflow harnesses on each architecture at source commit
`7c520b50f44e1286563693180311c6030afed877`.
[Publication](https://github.com/mingley/agent-video-workbench/actions/runs/37097667541)
verified qualification/archive/binary identities before uploading. The public
installer then downloaded that Ubuntu x86_64 release into a fresh Debian 13
installation and passed readiness and the expanded official MCP workflow.
The prepared cloud demo also retains a PQ original, its verified SDR delivery
and independent captioned variants. Cloud installation/start instructions were
exercised and saved; environment snapshot publication is a separate product action.

## Explicit support boundaries

Delivery is **Rec.709 SDR H.264/AAC**, up to one hour per output and 4096 pixels
per canvas axis. Linux x86_64 and ARM64 with persistent local filesystem locking
are qualified. Mac/Windows binaries, arbitrary consumer-agent accounts and
phone playback/appearance are not certified by these Linux tests.

Dolby Vision profile 8 with HDR10/HLG compatible base layer is recognized;
actual Dolby Vision footage is not in the generated qualification matrix.
Profile 5, unqualified BT.2020 non-PQ/HLG or Log transforms, ProRes/Log/Cinematic
metadata, HDR-output masters and GPU encoders need additional implementation
and sample qualification. This release does not advertise those paths.

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
