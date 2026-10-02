# Quality, performance and release gates

Status: proposed acceptance policy for F24 and every media-facing feature.
Targets below are product test decisions, not measured results. The existing
[candidate results](../../evaluation/README.md) cover only their generated cases.

## Verification report

Every exported artifact has a report with file hash, project/sequence revision,
preset/plan/toolchain identity, expected properties, measured properties and
named checks. Each check is `passed`, `failed`, `warning` or `not_run`, with
evidence and any tolerance. An unavailable check cannot count as a pass.
Editorial review is recorded separately from technical verification.

| Check | Acceptance policy |
| --- | --- |
| Decode | Full decode of final short outputs completes; decode errors fail delivery |
| Shape | Dimensions, display orientation, SAR, codec and pixel format match the selected preset |
| Video timing | Exact decoded frame count for fixed-rate short output; no unintended frame duplication/drop at cuts |
| Audio timing | No cumulative drift; seam alignment within one output frame on fixtures after accounting for documented codec priming/padding |
| Sources | Expected source intervals and protected coverage agree with the frozen edit map; source hashes remain unchanged |
| Caption integrity | Expected cues, fonts/glyphs, visible bounds and timing; no unresolved source bindings |
| Audio levels | Measurements meet the chosen profile's explicit integrated loudness/peak tolerances; no accidental doubled dialogue |
| Color | Expected transform chain and output tags; qualified real-phone reference review for supported HDR paths |
| Black/freeze/gap | Findings compared against intentional title cards, transitions, holds or gaps; unexpected intervals need resolution |
| Provenance | Artifact hash, input revision, preset, dependency versions and check results are present and mutually consistent |

Phone review remains necessary for story choice, crop/product visibility,
caption readability, speech recognition and acceptable color. Automated pixel
or OCR checks supplement those judgments; they cannot certify them by themselves.

For word timing, start with a proposed ±150 ms review tolerance on manually
aligned clean-speech fixtures and inspect every cut boundary. This is an ASR
qualification target, not a universal promise. Record word error rate per
language/noise condition, product-name corrections and timing failures. Define
supported language/model combinations from those results rather than advertising
every language a model lists.

## Fixture suite

| Group | Required cases |
| --- | --- |
| Synthetic deterministic media | Known frame colors/counters, tones/impulses, pauses, captions, alpha, rotation matrices, non-zero timestamps |
| iPhone originals | Portrait/landscape, H.264/HEVC, SDR and qualified HDR profiles, VFR/low light, 4K/60 fps, missing/silent audio |
| Editorial operations | Linked A/V trim/reorder, B-roll replacement, J/L cut, freeze/speed, transition handles, selective restoration |
| Text | Long lines, quotes/filter punctuation, product names, emoji/fallbacks, combining marks and each supported RTL/mixed script |
| Transfer/storage | Interrupted/resumed download, changed remote object, wrong hash, expired link, missing source, storage exhaustion |
| Durability | Lost response, two writers, process kill before/after commit, migration failure, interrupted backup/restore and late worker result |
| Agent behavior | Fresh session, stale preview, ambiguous IDs, unknown capability, pagination, failed render, old review after opening change |

Keep private media outside Git; version public generated fixtures and manifests
for permitted real samples. Record enough metadata to reproduce the chosen
pipeline without exposing private URLs or secrets. Candidate fixtures remain
useful, but application tests must exercise our actual Rust service/store/render
adapter and workers.

## Performance method

Use a named CPU-only Linux reference profile initially targeting 4 vCPU/8 GiB
RAM, plus the actual machine/OS/storage/tool versions. This is a proposed test
profile, not a measured minimum or a hosted provider specification. Measure:

- Cold and warm transfer/probe/index, transcription, proxy, preview and final render.
- Time to first inspectable frame/preview and latency after a caption-only edit.
- Peak process-tree RSS, scratch/durable disk, bytes transferred and cache hits.
- Encoding speed/quality and hardware/CPU transfers when acceleration is enabled.
- Failed/interrupted work, corrective tool calls and creator revision turnaround.

Run 2–5-minute, 30-minute and 60-minute sources, including a late-source clip.
Use repeated samples and publish median/tail results with workload details.
Avoid comparative claims when codecs, quality, caches or hardware differ.
Hardware support requires a successful encode and measured output; advertised
encoder names alone do not qualify a device.

Initial regression budgets after a baseline is established: investigate a
greater than 20% median latency or peak-RSS increase on identical workloads,
require no retranscription on style-only edits, and require memory growth to
remain bounded rather than proportional to decoded video length. These are
review gates, not automatic evidence that every smaller regression is acceptable.
Set user-facing latency targets only after measuring the real host and footage.

Hash dependencies for cache correctness. Tests must show both a valid reuse
(same source/recipe) and invalidation (changed font, model, preset or toolchain).
Byte-identical output is a narrow same-toolchain property; cross-platform
acceptance compares decoded timing/content and declared tolerances rather than
requiring identical compressed bytes from different encoders.

## Release qualification

Every supported platform gets a clean-install test, actual caption render,
project reopen, interrupted-job recovery and artifact retrieval. Every claimed
hosted bot gets a dated account-route test. Unknown quotas remain marked unknown.
Schema changes need migration fixtures from retained supported versions and a
backup/restore drill. A binary update must not alter old project decisions or
mislabel an old render as current.

Stable 1.0 requires a published input/platform/protocol matrix, reproducible
release manifests, documented known limits, no unresolved data-loss issue in
the supported workflow, and successful creator trials across conversation
resets. New optional tracking/color/provider features graduate independently;
their failure cannot break basic editing of projects that do not use them.

Cost reports separate measured local resources, known provider usage, configured
price estimates and unknown charges. Local ASR has CPU/storage cost even without
an API bill. Quality/performance choices must expose which backend/model and
output settings were actually used.
