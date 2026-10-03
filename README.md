# Agent Video Workbench

A Rust video tool for an external agent, available through CLI JSON and MCP
stdio. It owns immutable originals, editable projects, revision history,
analysis, persistent jobs and verified delivery. The agent chooses content and
uses its host's file tools to deliver results.

**Version 0.3.1: Linux x86_64 and ARM64, October 3, 2026.** Import local or
resumable HTTPS originals, convert PQ/HLG HDR to Rec.709 SDR, inspect source
frames/PTS/scenes/silence, transcribe locally, and compose captioned shorts.
Revise with selective source restoration, saved profiles/templates, B-roll,
independent format/language variants, audio ducking/loudness, reversible grades
and editable tracked crops. Freeze a batch and deliver MP4s, SRT/VTT, covers,
contact sheets and a local HTML review bundle. Catalog, retention, verified
backup/restore, relinking and supported-cut OpenTimelineIO interchange are included.

Start with [installation and MCP connection](docs/deployment.md), then run
[the first-edit walkthrough](docs/first-edit.md). The
[agent workflow](AGENT_GUIDE.md) covers subsequent edits and recovery. Download checksum-verified binaries from
[releases](https://github.com/mingley/agent-video-workbench/releases).
From a repository checkout, the native binary installer needs no Rust compiler,
Node, GUI or root (see deployment for installation without a checkout):

```sh
scripts/install-release.sh /path/to/new-bin 0.3.1
scripts/setup-media.sh /path/to/tools
/path/to/new-bin/avw --ffmpeg /path/to/tools/ffmpeg8/linux/ffmpeg \
  --ffprobe /path/to/tools/ffmpeg8/linux/ffprobe doctor
/path/to/new-bin/avw create /path/to/project --name "First short"
/path/to/new-bin/avw agent-guide
```

Building uses latest stable Rust, rustfmt and strict Clippy. Runtime uses FFmpeg
8 with the required caption/HDR/audio filters and an imported licensed TTF font.
Optional `scripts/setup-asr.sh PROVIDER_DIRECTORY` prepares pinned whisper.cpp
and checksum-verified English weights. Other analysis providers use an explicit
versioned process contract. No proprietary model account is required.

SQLite commits edits, history and retry outcomes together. Workers freeze input
revisions, fence ownership, bound child memory/files/logs, and support cancellation,
crash recovery and retries. Exports publish only after full decode and technical
QC. Originals and historical references survive cache collection.

The [acceptance matrix](docs/implementation-status.md) records actual generated
media, official MCP SDK, native CI, local ASR, short 4K/60fps and one-hour-source
tests. Delivery is SDR H.264/AAC. Real phone appearance and additional scripts,
camera profiles, operating systems and hosted agent products need their own
qualification; the tool reports unsupported rendering paths explicitly.

| Document | Purpose |
| --- | --- |
| [First edit](docs/first-edit.md) | Tested import-to-delivery CLI/MCP walkthrough |
| [Interface reference](docs/agent-api.md) | Current envelopes, revisions, jobs and verified artifacts |
| [Agent guide](AGENT_GUIDE.md) | Source-based edits, profiles, analysis, revisions and delivery |
| [Deployment](docs/deployment.md) | Published installation, MCP, providers, storage and maintenance |
| [Development](DEVELOPMENT.md) | Latest stable Rust, strict checks and functional qualification |
| [Implementation status](docs/implementation-status.md) | Executed evidence and precise support limits |
| [Product plan](docs/product-plan.md) | Broader creator milestones |
| [Backlog](docs/implementation-backlog.md) | Feature completion and remaining qualification gates |
| [Architecture](docs/architecture.md) | Rust boundaries, exact time and durable state |
| [Foundation decision](docs/decision.md) | Pinned MIT AgentCut libraries |
| [Evaluation](evaluation/README.md) | Current checks and historical candidate comparison |
| [Service schema](specs/schemas/service-request.schema.json) | Generated CLI/MCP request envelope |

Agent Video Workbench is [MIT licensed](LICENSE), copyright 2026 Michael Ingley.
AgentCut's required [third-party MIT notice](licenses/agentcut-MIT.txt) is kept
separately. FFmpeg, fonts and models retain their own licenses. Keep private
footage, credentials and model weights outside Git.
