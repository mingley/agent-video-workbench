# Implementation backlog beyond the MVP

Status: proposed work; completion must be established by code and acceptance
evidence. Milestones and feature IDs come from the [product plan](product-plan.md).
The original [roadmap](roadmap.md) retains the first end-to-end workflow gates.
The [status inventory](implementation-status.md) describes the existing SDR
prototype. The slices below extend/harden it; real phone and hosted-agent
acceptance remain open.

## Next implementation slices

| Work item | Depends on | Concrete deliverable and acceptance |
| --- | --- | --- |
| N01 — Store/protocol completion | Existing prototype | Compact status, paginated history, semantic diff, created-ID results and query-by-idempotency-key; reopen/lost-response/concurrent writer tests through the actual CLI |
| N02 — Harden managed ingest | Existing import, N01 | Extend stream/hash/copy/probe with persisted partial/ready states and transfer recovery; interrupted import reconciles and never overwrites a source |
| N03 — Qualify the renderer adapter | Existing adapter, N02 | Expand the corrected display-matrix path to real phone orientations and timing fixtures; upright rendering exactly once with recorded color decisions |
| N04 — Recoverable render jobs | Existing synchronous jobs, N03 | Add bounded diagnostics and automatic attempt reconciliation; kill/restart preserves the old final and recovers the frozen-revision job |
| N05 — Source-based revision loop | N01–N04 | Captioned one-short draft, omission records, selective restore, style/opening revision and undo; actual phone playback and unchanged original hash |
| N06 — Source inspection | N02, N04 | Local/imported ASR, frame/silence/scene indexes, searchable words, proxies and protected intervals; known sentence/demo found and preserved after pause edits |
| N07 — Creator clip set | N05, N06 | Three named shorts with saved style and explicit framing/HDR policy; creator's requested changes affect the intended outputs after a fresh session |
| N08 — First install/host route | Existing native packaging, N03, N04 | Qualify the bundle/dependencies and one real bot-route trial; install, retrieve original, render, return preview and reopen persistent project |

Each slice should be reviewable independently. A Rust change updates status and
the matching draft contract, adds behavioral tests where needed, and links its
actual acceptance evidence. Do not mark a milestone complete because its
documentation, dependency declarations or command names exist.

## Product epics

| Epic | Features / milestone | Dependencies | Acceptance boundary |
| --- | --- | --- | --- |
| E01 — Durable editorial history | F01 / M0–M2 | N01, N05 | Selective restoration retains unrelated edits; diffs expose source and style changes; transaction faults cannot split head/history/outcome |
| E02 — iPhone ingest qualification | F02 / M1 | N02, N03 | Supported SDR/HDR/HEVC/VFR matrix passes; unsupported profiles get precise capability results; transfer retry checks remote identity |
| E03 — Inspectable sources | F03 / M1 | N06 | Bounded transcript/frame APIs, explicit partial coverage and cache reuse; ASR corrections survive reanalysis as a separate layer |
| E04 — Caption engine | F04 / M1 | N05, N06 | Source-linked cue remapping, line/layout checks, actual font assets and matching burned/sidecar output |
| E05 — Framing variants | F05 / M1–M2 | N03, E04 | Vertical/square/landscape crops are independent and preserve selected content; safe-area changes are reviewable |
| E06 — Multi-source timeline | F07, F08 / M2 | N07, E01 | Three recordings plus B-roll/logo, linked speech and J/L cuts; replace/reorder leaves intended A/V/caption relationships intact |
| E07 — Creator library | F10, F23 / M2 | E01, E04 | Searchable projects/profiles, pinned styles/fonts/assets and explicit upgrade diff; old output remains reproducible |
| E08 — Hooks and review | F09, F20 / M2 | E01, E05 | Named alternatives, comparison bundle and artifact-based comments; old feedback resolves after an opening change |
| E09 — Repeatable clip sets | F11 / M2 | E06, E07 | Template slots validate new bindings/duration; expansion is a normal atomic batch; no leaked prior-project source IDs |
| E10 — Audio finishing | F06 / M2 | E06, N04 | Bus gain, fades, ducking and optional cleanup; profile loudness/peak checks plus speech A/B review; no duplicate audio route |
| E11 — Batch delivery | F12 / M2 | E05, E08, N04 | Frozen aspect/language output set with independent job outcomes, cover/sidecar files and complete manifest |
| E12 — Language/accessibility | F13 / M3 | E04, E11 | Qualified script/font matrix; corrections/translation alignment; language variants preserve original speech and separate review state |
| E13 — Workers and resource limits | F16 / M3 | N04, E11 | Lease generation fences late workers; cancel/retry/priority/space budgets; interrupted batch resumes compatible stages |
| E14 — Portable projects | F17 / M3 | E01, E07, E13 | Consistent bundle/restore on a fresh host; verify media/font hashes; missing objects relinkable; GC respects history and job leases |
| E15 — CLI/MCP conformance | F18 / M3 | N01, E13 | Shared schemas/capabilities/error semantics; CLI and MCP produce equivalent durable outcomes; tool calls remain bounded |
| E16 — Release installation | F19 / M3 | N08, E14, E15 | Qualified OS/CPU archives, notices/manifests, offline option and safe tool upgrade/repair; actual host matrix |
| E17 — Long-input qualification | F24 / M4 | E02, E13, E16 | Published cold/warm 60-minute results, late-source preview test and bounded resource use; style edits reuse analysis |
| E18 — Stable compatibility | F01, F18, F19 / M4 | E01–E17 supported subset | Schema migrations/backups, protocol conformance, release/rollback drills and repeated creator trials pass |
| E19 — Assisted reframing | F14 / M5 | E05, E17 | Manual motion first; tracker returns editable proposals with version/confidence/fallback; product remains visible on qualified samples |
| E20 — Color/HDR delivery | F15 / M5 | E02, E17 | Explicit color pipeline, reversible grading and qualified HDR output; no metadata-only claim of correct HDR |
| E21 — Interchange | F21 / M5 | E06, E18 | Supported-subset round trips preserve timing/source bindings; every unsupported feature appears in a loss report |
| E22 — Provider extensions | F22 / M5 | E13, E15, E18 | Versioned process contract, cancellation/resource limits and malformed-output tests; providers cannot bypass project mutations |

M2 editing work and M3 portability can overlap after their prerequisites. The
stable compatibility gate evaluates only the features/platforms actually
included in that release; an optional feature stays unavailable until its own
acceptance tests pass. Minimum job recovery and transactional integrity are
required earlier for creator alpha.

## Investigation items with decision criteria

| Question | Small experiment | Decision / fallback |
| --- | --- | --- |
| Does AgentCut remain a useful library boundary? | Complete N03–N05 and measure adapter/patch scope | Keep it if edits/rendering remain reusable; revisit OpenReelio or a narrow FFmpeg model if it requires a broad engine rewrite |
| Can the chosen hosted bot execute jobs reliably? | N08 with actual original transfer and a conversation reset | Direct CLI when qualified; supported remote tools otherwise; record unsupported products explicitly |
| Is local ASR practical on the available CPU? | Time/measure representative speech and review words/cuts | Choose model/settings from evidence; allow imported/configured remote transcripts without making them mandatory |
| Do proxies or source-range seeks help this workload? | Compare cold/warm late-source previews and final quality | Use the cheaper correct path per source; avoid obligatory full mezzanines |
| Is assisted cropping useful enough? | Creator compares static crops, manual keyframes and tracked proposals | Ship manual control first; tracking is optional until it improves results without excessive review |
| Which formats/scripts should 1.0 claim? | Run the media/text/platform matrix on actual intended samples | Publish a bounded supported subset and explicit unavailable reasons |

No calendar estimates are committed here. Track implementation size and trial
results from the next slices before assigning dates to the larger milestones.
The broader plan should grow from successful creator workflows, with every
new capability preserving the same source/revision/recovery guarantees.
