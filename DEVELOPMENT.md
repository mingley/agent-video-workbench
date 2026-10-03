# Develop Agent Video Workbench 0.2

The Rust application provides durable local projects, a typed CLI/JSON API,
MCP stdio, source inspection, optional local transcription, captioned named
outputs and persistent rendering workers. Read [the agent guide](AGENT_GUIDE.md)
for the editing workflow and [deployment](docs/deployment.md) for installation,
MCP configuration and restart behavior. The qualified scope is in
[implementation status](docs/implementation-status.md).

## Prerequisites and required checks

Use latest stable Rust (`rustup update stable`); the manifest currently requires
Rust 1.99+. The stable toolchain file includes rustfmt and Clippy. A C compiler
builds bundled SQLite. Runtime needs FFmpeg/ffprobe 8 with libx264, AAC and
caption filters, plus an imported, licensed TTF font. Linux x86_64 on persistent
local block storage is the qualified route. No GUI, GPU, Node or model service
is needed to run the binary.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --release --locked
python3 scripts/check-schema.py target/release/avw
```

Format changes with `cargo fmt --all`. Never suppress warnings to pass Clippy.
The generated request contract is
[service-request.schema.json](specs/schemas/service-request.schema.json).
Regenerate it from the `result` in `avw schema` when request types change.
`describe OPERATION` provides examples and the domain property registry;
`apply --dry-run` checks actual parameters, time arithmetic and project policy.
The older draft schemas remain design documents.

## Media and agent checks

On this Linux route, `scripts/setup-media.sh /path/to/tools` installs a
checksum-verified FFmpeg 8 backend without root. FFmpeg remains an external
runtime with its own license. Substitute its paths below. Each test output
must be a new directory outside the checkout.

```sh
python3 evaluation/application_smoke.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --font /path/to/font.ttf --output /path/to/new-application-run
python3 evaluation/job_smoke.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --project /path/to/new-application-run/backup --output /path/to/new-job-run
python3 evaluation/media_matrix.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --output /path/to/new-matrix-run
npm ci --ignore-scripts --prefix evaluation
node evaluation/agent_smoke.mjs "$PWD/target/release/avw" \
  /path/to/ffmpeg /path/to/ffprobe /path/to/font.ttf \
  /path/to/new-agent-run "$PWD/evaluation/node_modules/@modelcontextprotocol/sdk"
```

The application regression creates a two-minute source, a captioned 30-second
draft and a 31-second revision, then checks restore/reopen, byte-identical undo,
source corruption rejection, protection, rotation and portable backup. It
intentionally corrupts its original project at the end; use its intact `backup`
for the worker regression. That regression kills an actual worker, checks child
termination and reconciliation, retries, cancels and preserves the earlier
verified output. The matrix checks four rotations against upright pixels, HEVC
SDR, VFR and missing audio; it converts 10-bit PQ/HLG and accepts nonzero stream starts with decoded reference comparisons.
The official MCP SDK test produces three independent captioned outputs, checks
CLI/MCP replay equivalence, cached inspection, conflicts, policy removal,
artifact verification and backup reopening. Node is only a test dependency.

Optional local ASR needs CMake and C/C++ to build pinned whisper.cpp v1.9.4.
`scripts/setup-asr.sh /path/to/providers` verifies source revision and tiny.en
model SHA-256. Retain the provider build directory and its shared libraries.
It downloads no private media; model weights remain outside Git.

```sh
python3 evaluation/asr_smoke.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --whisper /path/to/providers/whisper/build/bin/whisper-cli \
  --model /path/to/providers/whisper/models/ggml-tiny.en.bin \
  --model-sha256 921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f \
  --output /path/to/new-asr-run
python3 evaluation/long_input.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --font /path/to/font.ttf --output /path/to/new-hour-run
```

ASR speech generation uses `/usr/bin/ffmpeg` with the `flite` filter. The ASR
adapter itself uses the configured FFmpeg backend. The long-input benchmark
uses Linux `/proc` sampled process-tree RSS and a low-resolution generated
one-hour source. It does not establish 4K camera performance or hard host
resource reservations. Every harness preserves a current-run summary and
command ledger. Inspect outputs as well as test exit status.

## Release archive

`scripts/package.sh /path/to/new-release-directory` runs required Rust/schema
checks and creates a versioned native archive, installer, licenses, dependency
license declarations, compiler/platform/commit manifest and SHA256SUMS. Verify
and extract the archive; run `avw/install.sh /path/to/new-user-bin`. The installer
verifies the binary and refuses to overwrite an existing installation. Cargo,
Node and root are unnecessary on the destination host. The bundle excludes
FFmpeg, fonts and models. Only the native Debian 13 x86_64 build has been
installed and exercised here; other platform builds need qualification.

CI runs the Rust/schema, generated application, worker, media and official MCP
checks, then retains the native bundle. Local ASR and one-hour measurements
are separate release qualification checks. Product roadmaps and historical
candidate results describe additional work; they are not runtime contracts.
