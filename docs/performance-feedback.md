# Export verification and first-edit improvements

## Local follow-up: share the review-sheet decode

Exports now generate their contact sheet during the full verification decode.
SDR masters supply it directly; HDR/lossless masters use their SDR review video's
verification pass. Sampling finishes after four tiles while the validation sink
continues to EOF. There is no separate contact-sheet decode. One-to-three-frame
exports now list only existing sampled frames in the sheet manifest.

Against the previous fix (`9a484d2`), three alternating runs on the same generated
60-second 720p source / 30-second output measured **16.22 → 14.14 seconds median
(12.8% less elapsed time)**. The isolated verification-plus-sheet subprocesses
fell from **4.36 → 2.37 seconds**. Decoded video/audio and PNG sheet hashes match.
[Full follow-up measurements](../evaluation/service-results/review-sheet-performance.json)
retain all runs, stage times, hardware and binary hashes. These are separate
runs from the initial comparison below; do not combine their percentages into
an overall speed claim. The rendering implementation was measured before the
final staged-font-size guard, which affects imports only.

Reproduce with `evaluation/feedback_performance.py --review-sheet` and the same
arguments below, using `9a484d2` as the before binary. New render manifests include
`timingsSeconds` for original verification, planning, encoding, audio finishing,
master verification, SDR review and delivery. In the measured case encoding still
takes about 11 seconds. `totalBeforePublish` excludes final manifest writes,
file/directory sync and publication; it is not end-to-end request latency.

Imports now reject short/blank keys, new stale-revision requests and oversized
fonts/LUTs before copying or probing. Committed retries still reach the existing
full request comparison, preserving old-revision replay and changed-intent
conflicts. A staged-size check also catches fonts/LUTs that grow during copying.
Import checks reject existing redirected cache/original directory slots and
symbolic-link leaves for managed originals. These are static checks, not isolation
against concurrent filesystem replacement.

Local validation passed 176 Rust tests, two explicitly invoked real-media tests,
formatting, strict Clippy, the release build and generated-schema check. The
application and worker regressions passed 30/31-second exports, exact undo,
corruption rejection, owner death, retry and cancellation. Color/memory testing
rendered 393 full-HD HLG frames under the 4GiB child limit. PQ/HLG preservation
passed native 10-bit pixel equality, metadata and audio/light timing; the full-HD
repeated-cut case and SDR review videos passed too. Very short/fractional-rate
sheets match the old PNG bytes, and damage after the last sampled tile is refused.
The new import tests cover early failures, replay, changed-intent rejection and
redirected paths. See the measurement JSON's `validation` field.

This iteration uses local tests and `[skip ci]` for the push. Its checks do not
establish native ARM64 qualification for the new head. The initial CI runs on
`9a484d2` remain historical evidence for that exact commit.


## Initial iteration: scope and reproduction

The engine has no ten-second import limit. The short ranges in the introductory
examples were editorial selections. This change makes that explicit and retains
the existing one-hour output limit. The generated application regression also
exports 30- and 31-second revisions; the performance fixture imports 60 seconds
and exports two 15-second cuts as one 30-second video.

The previous verifier fully decoded the export with FFmpeg to detect corruption,
then decoded it again with `ffprobe -count_frames`. Verification now counts the
frames during the strict FFmpeg decode, with timestamp passthrough to prevent
frame duplication/drop, and uses ffprobe only for stream/container metadata.
It requires a final progress record and preserves the existing `nb_read_frames`
manifest field, adding `frameCountMethod` to identify its provenance. Codec,
geometry, rate, color, HDR metadata, audio duration and file-hash checks remain.
Encoding presets, quality, original-media verification, cancellation and child
resource limits are unchanged. This does not introduce hardware encoding.

The initial iteration's three-run median total export time fell from **18.21s to
15.60s (14.3% less elapsed time)**. The isolated
verification subprocess median fell from **5.31s to 2.52s**.
Decoded video and audio hashes match exactly.

## Reproduce the measurement

Build the baseline from main commit `2248929` and the candidate in release mode,
then run without competing builds or render jobs:

```sh
python3 evaluation/feedback_performance.py \
  --before /path/to/baseline/avw --after /path/to/candidate/avw \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --output /path/to/new-benchmark-directory
```

The harness generates 60 seconds of 1280×720, 30fps SDR testsrc2 and sine audio,
imports it separately into each binary, and alternates three render runs per
binary. It checks 900 output frames, unchanged source bytes and equal independently
decoded video/audio hashes. It separately measures the verification subprocesses
on the same delivered file. Hashing and policy checks are excluded from that
isolated subprocess measurement, but included in total export time.

See [the recorded measurements](../evaluation/service-results/feedback-performance.json)
for per-run timings, binary hashes and hardware. This is a synthetic CPU test on
Linux x86_64, AMD EPYC 9V74, a four-CPU cgroup quota and 16GiB container memory;
the existing per-child 4GiB limit remains. It uses Rust 1.99.1 and FFmpeg 8.0.1.
Shared-host scheduling and cache effects vary. It does not measure 4K camera
footage, ASR, GPU performance or physical HDR appearance. Encoding remains a
substantial cost; Rust application code does not make codec work instantaneous.

## Initial feedback compatibility and validation

- README declares the manifest's Rust 1.99 minimum and the existing stable
  toolchain file; 1.99.1 is the compiler used here. Rust 1.99.0 was not separately
  exercised.
- CLI help and the guide recommend 8–200 ASCII-character idempotency keys and
  explain retry reuse. Existing validation semantics remain compatible.
- First-edit targets 0.4.0, distinguishes published binaries from the new CLI
  presentation option, and explains the 360×640 initial canvas versus the
  independent 1080×1920 compose defaults.
- New-project `createdBy` names AVW and its build version. Existing snapshots
  and the upstream AgentCut project schema identifier remain unchanged.
- FFmpeg 8.0.1 is the qualified backend; `doctor` is capability-based rather
  than a version gate. FFmpeg 7.1.5 was not tested here.
- `--result-pointer` extracts a JSON Pointer relative to the CLI result, so
  scripts capture batch/child IDs without manual copying. Errors remain
  structured. Invalid syntax fails before execution; a missing field after
  execution returns the successful result with an explicit presentation error.
  MCP responses and the service request schema are unchanged.

Validation: 171 ordinary Rust tests, one explicitly invoked real-media test,
formatting, strict Clippy, release build and generated-schema check passed.
The real-media test checks a 12.012-second 30000/1001fps file, incorrect expected
frame count and truncated bytes. Generated HDR preservation checks pass native
10-bit pixel equality, metadata, audio/light timing and SDR review generation.
The application smoke checks 30/31-second exports, undo, original corruption and
rotation. The updated first-edit shell blocks complete both five-second renders,
automatic ID capture, delivery and backup restoration; both manifests contain
150 frames and the captioned contact sheet was inspected. Local Markdown links
and `git diff --check` pass. These are local x86_64 checks, not a new ARM64/native
release qualification. No release or deployment is included.
