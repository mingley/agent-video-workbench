# Initial implementation roadmap and broader milestones

Each phase delivers a usable check of the workflow. The current Rust SDR
prototype is described in [DEVELOPMENT.md](../DEVELOPMENT.md) and the
[implementation inventory](implementation-status.md). Its synthetic Linux checks
cover transactional storage, immutable import, captioned renders, rotation,
revision restoration, source protection, durable render records and portable
media backup. Real phone/host acceptance remains open; none of phases 0–5 is
claimed complete. These initial phases feed the [broader product milestones](product-plan.md),
with an [expanded backlog](implementation-backlog.md) for the complete creator
workbench. Build usable slices while retaining that larger product design.

| Phase | Work | Acceptance criteria |
| --- | --- | --- |
| 0. Foundation and host fit | Pin candidates; reproduce current findings; test a corrected rotation path; build a no-sudo binary/tool bundle; choose one available hosted bot for a trial | On a clean CPU Linux environment, download/install and run metadata + a tiny caption render without Cargo/Node/GUI. On the chosen bot account, verify shell or custom-tool access, a supported durable path, leave/reopen recovery, and retrieving the rendered original artifact. Record actual quotas/limits. Fix or isolate rotation and do not use AgentCut's best-effort journal as authority. |
| 1. Small persistent editing loop | Rust CLI/core, SQLite revision transaction, content-addressed original, source ranges, cut/crop/caption, preview/render, history/diff/undo, idempotent requests | One 2–5-minute iPhone original yields one 30–60-second vertical draft. Fresh process restores an omitted interval, shrinks captions, changes opening, renders revision 2 and undoes/redoes it. Source hash unchanged; exact decoded video length; A/V checked; snapshot + history + request outcome commit together. |
| 2. Agent inspection | Source probe/PTS index, proxy, silence/scene results, frames/sheets, local ASR/import adapters, source-word IDs and search, protected annotations | Agent can locate a known sentence and product demonstration through tool results. Pause suggestions preserve protected non-speech footage. Caption mapping stays correct after reorder/restore; repeat inspection hits cache; ASR failure does not corrupt the project. Word timing/caption accuracy is reviewed on phone footage. |
| 3. Creator MVP | Three named outputs, saved caption profile, per-shot static framing/letterboxing, overlay, gain/loudness, versioned SDR 1080x1920 H.264/AAC 30 fps preset, explicit HDR/VFR handling | A 10–30-minute recording yields up to three 30–60-second previews/finals. Creator requests the example revisions; agent finds the existing output/source IDs and changes only the intended content. Demonstration preserved; captions/readability/framing checked on an iPhone; HDR and SDR samples match the supported matrix. If footage lacks enough material, return that fact rather than inventing filler. |
| 4. Durability and packaging | Persisted jobs, bounded logs, cancel, interrupted-stage retries, backup/restore/relink, cache retention, release archives/checksums/notices; thin MCP where required | Kill a render and restart; prior final survives and job is reconcilable. Lost-response retry does not duplicate edits/jobs. Two writers with one base revision cannot both commit. Disk-full/history-write faults commit neither state nor success. Restore on a fresh machine reproduces editable state/history. Install and test on each claimed hosted bot; document unsupported routes. |
| 5. Long inputs and measured acceleration | 60-minute/4K fixtures, resource measurement, targeted decode, cache scheduling; benchmark available hardware; optional segmented export | Report cold/warm ingest, ASR, preview and export timing, peak RSS and disk. Memory is bounded rather than proportional to decoded video length; a caption change does not retranscribe. Hardware claims use actual encoder tests and measured quality/time; no unreported CPU substitution or segment A/V seams. |

**Build first:** the phase 1 one-short loop, preceded by the phase 0 media/host
gates. Keep phase 1 to the smallest complete sequence:

1. `doctor` and generated caption-render smoke check.
2. Import the original and persist its hash/metadata in a project.
3. Apply a supplied timestamped cut list/transcript, crop, and caption style in
   a validated transaction. A manual transcript is acceptable for this slice.
4. Render one preview and publish its revision/source-range manifest and frame
   sheet. Independently decode and measure the output.
5. Exit, reopen, locate the omitted sentence/range through history, apply the
   restoration/style/opening batch, render, undo, and redo.
6. Repeat from a clean hosted environment or supported external media worker.

## Beyond the first workflow

| Milestone | Additional scope | Acceptance |
| --- | --- | --- |
| M2 — Daily creator workflow | Multi-source editing/B-roll, richer A/V edits, reusable profiles/templates, alternate hooks, review bundles and multi-aspect delivery | Assemble three originals over continuous dialogue; preserve named variants; apply profile upgrades explicitly; resolve feedback against an older preview |
| M3 — Portable beta | Multilingual captions, worker scheduling, CLI/MCP parity, portable projects and qualified installation routes | Restore on a fresh host, recover an interrupted batch, verify script/font output, and demonstrate equivalent tool outcomes across supported routes |
| M4 — Stable 1.0 | Versioned compatibility guarantees, migrations, long-input measurement and reproducible release bundles | Pass the supported input/platform matrix, backup/upgrade drills and repeated creator trials; publish measured resource budgets and known limitations |
| M5 — Advanced releases | Assisted reframing, richer color/HDR delivery, timeline interchange and provider extensions | Each optional capability passes its own source/timing/quality fixtures and reports unavailable cases precisely |

M2 and M3 can overlap where dependencies allow. Transactional integrity and
minimum render recovery are required before creator alpha; later milestones
extend their coverage. The feature catalog and issue-ready epics define what
each milestone actually includes, instead of treating all future editing ideas
as required for the first release.

The application implementation should have meaningful tests for atomic
rollback, lost-response retries, concurrent writers, crash recovery, VFR time
mapping, protected intervals, A/V linking, literal caption text and render
dimension/color policy. The synthetic candidate evaluation provides media
fixtures but does not substitute for these future storage/host tests.

For the initial phone matrix, obtain representative originals with permission:
portrait and landscape SDR; orientation in a MOV display matrix; HEVC 10-bit
HLG/HDR; VFR/low-light; silent footage; footage with interruptions and long
pauses. Also test missing audio, unusual stream starts, long names/unicode,
caption quotes/emoji, insufficient disk and interrupted transfer. Synthetic
fixtures make assertions repeatable; original iPhone recordings reveal the
ingest and appearance issues that generated H.264 cannot prove away. Keep
private samples outside Git; retain only permitted derivatives/manifests.

The first creator trial succeeds when she can review a draft and request the
example changes naturally, with the agent revising the same project after a
conversation restart. Evaluate the bot's actual tool use as well as the editor:
number of corrective tool calls, wrong IDs/units, stale revisions, transfer
failures, edit/preview latency, and whether it reviews the rendered file.
An install guide or agent skill may help; it is not a new agent harness.

**Cost accounting:** the editor/license is free; local FFmpeg and optional
local ASR require compute and storage; a hosted bot has its own subscription or
usage cost; remote ASR/VLM can incur per-call costs; storage/download delivery
can incur capacity/egress costs. Record actual model/provider usage when exposed
and mark unknown charges as unknown. Do not invent a per-video price from a
six-second synthetic test. A remote worker is optional infrastructure with
its own bill, not a requirement hidden behind “open source.”
