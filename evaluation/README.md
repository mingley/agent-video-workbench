# Application qualification

The 0.3 application checks and exact commands are in
[DEVELOPMENT.md](../DEVELOPMENT.md). The current scope and evidence are in
[implementation status](../docs/implementation-status.md) and
[service qualification results](service-results/qualification.json).
`application_smoke.py`, `job_smoke.py`, `media_matrix.py`, `agent_smoke.mjs`,
`asr_smoke.py`, `long_input.py`, `studio_smoke.py`, `audio_smoke.py`,
`transfer_smoke.py`, `analysis_smoke.py`, `timeline_smoke.py`, `highres_smoke.py`
and `combined_media_smoke.py` drive the Rust application with generated
media. New output directories must be outside Git; model weights and private
footage must never be checked in. Node and the locked official MCP SDK are test
clients, not application runtime dependencies. `browser_smoke.mjs` uses locked
Playwright and an installed Chromium to verify the delivered HTML's playback,
seeking, links and desktop/phone layouts. This is viewport emulation, not physical
phone or iOS Safari qualification.

The rest of this document records a separate historical candidate comparison.
Its media, assertions and known failures do not describe 0.3 application
qualification.

# Reproduce the candidate edit

This is a research harness, not the new editing application. Python is used
only to generate fixtures and drive unmodified upstream executables. The
planned application's core, state and execution remain Rust.

[Pinned candidates](candidates.lock.json) record the source revisions used.
Builds and execution were tested on Apple Silicon macOS 26.7 with Rust 1.98.1
and FFmpeg/ffprobe 9.0.2. No upstream test suite or published installer was run.
The harness needs Python 3, three prebuilt CLIs, FFmpeg 8+ with `libx264`, AAC,
`drawtext` and `subtitles`/libass, and a local TTF font. Other platforms and
toolchain versions may produce different bytes or expose additional failures.

Clone into a disposable directory outside this repository. Full clones make
the historical commit available without assuming it remains the branch head:

```sh
git clone https://github.com/wuwangzhang1216/agentcut.git
git -C agentcut checkout --detach 20ecdffc9d770bc280b294ee00414cafe4ce36ed
git clone https://github.com/MatheusBBarni/agent-video-editor.git ave
git -C ave checkout --detach 71a75bda2d9289f9dc0ace802ec274f0f580bb8f
git clone https://github.com/openreelio/openreelio.git
git -C openreelio checkout --detach 13993f3f824081f1d83204aa0c08f0288ce8e075
```

Run the corresponding command in each checkout:

```sh
# agentcut/
cargo build --release --locked -p agentcut-cli
# ave/
cargo build --release --locked
# openreelio/
cargo build --release --locked -p openreelio-cli
```

Use normal Rust native build prerequisites. OpenReelio's default CLI Whisper
feature additionally needs its native build toolchain (C/C++, CMake/Clang as
appropriate); it does not require the frontend build. Check pinned upstream
build documentation for the target OS. Build downloads need network access.
On this Mac the required FFmpeg was Homebrew's keg-only `ffmpeg-full`, used by
explicit path; the slim `ffmpeg` formula lacked caption filters. Do not assume
any package called FFmpeg contains the required filters/codecs.

From the evaluation parent directory, invoke the harness with paths to the
three binaries. Replace the workbench path and choose a font available on your
machine. `--output` must name a directory that does not already exist.

```sh
python3 /path/to/agent-video-workbench/evaluation/smoke.py \
  --agentcut ./agentcut/target/release/agentcut \
  --ave ./ave/target/release/ave \
  --openreelio ./openreelio/target/release/openreelio-cli \
  --ffmpeg /opt/homebrew/opt/ffmpeg-full/bin/ffmpeg \
  --ffprobe /opt/homebrew/opt/ffmpeg-full/bin/ffprobe \
  --font /System/Library/Fonts/Supplemental/Arial.ttf \
  --output ./smoke-new
```

The evaluated fixture is 12 seconds of generated H.264/AAC: red intro at
0–3 s, grey pause at 3–7 s, green demonstration at 7–10 s, and blue ending
at 10–12 s. A central white rectangle stands in for the product; a 440 Hz
tone is silent during the pause. Captions are supplied manually. Neither
the restoration nor the captions tests speech recognition or content choice.

The harness keeps the intro/demonstration, crops vertically, adds captions,
renders, fully decodes the result and checks decoded frame counts. AgentCut
also gets invalid-batch, retry/conflict, cache, journal-failure and
restore/style/opening/undo/redo checks. ave gets silence detection and an
in-place-write rejection. OpenReelio gets linked A/V trims and a missing-project
error. A generated rotation-tagged MOV is tested independently. Expected
error responses are recorded; a completed run does not mean all candidates
passed every capability gate.

## Checked-in results

| Artifact | Purpose |
| --- | --- |
| [Summary](results/summary.json) | Measured dimensions, frames, durations, hashes, errors and limits |
| [Command ledger](results/commands.json) | All 71 commands with arguments, stdout/stderr and exit codes |
| [Draft](results/draft.mp4), [revision](results/revision.mp4) | AgentCut's 6 s draft and 7 s restored revision; generated public media |
| [Contact sheet](results/draft-sheet.png) | Intro and demonstration after the cut/caption render |
| [Draft frame](results/draft-frame.png), [revision frame](results/revision-frame.png) | Caption-size change on the retained demonstration |
| [ave frame](results/ave-draft-frame.png), [OpenReelio frame](results/openreelio-draft-frame.png) | Inspect actual captioned outputs; OpenReelio used a landscape preset |
| [Rotation reference](results/rotation-reference.png) | FFmpeg's upright display of the rotated source |
| [Rotation failure](results/rotation-failure.log) | Unmodified AgentCut render log |
| [Rotation diagnostic](results/rotation-diagnostic.json), [frame](results/rotation-diagnostic.png) | Supplemental direct-FFmpeg run with autorotation disabled; not an upstream fix |

Candidate builds and system font files are not redistributed. Absolute binary,
run-directory and font paths in the command ledger are replaced by `$AGENTCUT`,
`$AVE`, `$OPENREELIO`, `$FFMPEG`, `$FFPROBE`, `$RUN` and `$FONT`. `$TEMP` marks
ephemeral scratch paths in the checked-in copy. These are explanatory tokens,
not a ready-to-execute shell script. Requests contain generated IDs and no
private footage or model credentials.

The supplementary rotation diagnostic was made after the complete harness
run. To reproduce it, request a plan in your generated run directory:

```sh
/path/to/agentcut --json --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  render plan rotation.agentcut.json --output rotation-workaround.mp4 \
  --save rotation-plan.json --include-backend-args
```

Read the saved `args` array as data, insert `-noautorotate` immediately before
the `-i` for `rotated.mov`, replace the final argument with a fresh output
path, and invoke FFmpeg with that argument vector. The recorded diagnostic
contains the exact vector used. Independently probe/decode it; this run yielded
360×640, 90 frames and 3.0 s, with the reference orientation. Upstream source
and the main harness are unchanged. A production correction still needs
orientation regressions across modes and toolchains.

No real iPhone footage, HEVC/HDR/VFR sample, ASR model, long-input benchmark,
GPU encode, hosted bot or custom MCP connection was tested. The
[research](../docs/research.md), [host plan](../docs/host-compatibility.md), and
[backlog](../docs/roadmap.md) distinguish those remaining gates from the small
cases actually verified here. Short synthetic elapsed times are not product
performance or cost estimates.
