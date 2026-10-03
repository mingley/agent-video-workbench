# Agent Video Workbench

A Rust video tool for an external agent. It owns original media, editable
projects, revision history, analysis, background jobs and verified exports.
The agent owns conversation, content choices and file delivery. A GUI and a
proprietary model service are unnecessary.

**Version 0.2: local Linux CLI and MCP service, October 3, 2026.** Import footage,
inspect source frames/scenes/silence, import a reviewed transcript or run local
Whisper, compose independent named captioned shorts, render in the background,
revise after restarting, and reopen portable backups. SQLite commits edits,
history and retry outcomes together. Render workers support cancellation,
crash recovery and retries; completed exports include technical QC, source
snapshots and contact sheets.

Start with [installation and MCP connection](docs/deployment.md) and the
[agent workflow](AGENT_GUIDE.md). The native installer needs no Rust compiler,
Node, GUI or root. Building from source uses latest stable Rust:

```sh
cargo build --release --locked
./target/release/avw --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe doctor
./target/release/avw create ../avw-project --name "First short"
./target/release/avw agent-guide
```

Use FFmpeg/ffprobe 8 with libx264/AAC and an imported licensed TTF font.
`scripts/setup-media.sh TOOL_DIRECTORY` prepares the qualified Linux backend.
Optional `scripts/setup-asr.sh PROVIDER_DIRECTORY` installs pinned local English
Whisper with checksum-verified weights. Paths, transcripts, jobs and exports
are available through the same typed CLI/JSON and MCP operations.

The qualified route is Debian 13 x86_64 and local persistent storage, delivering
SDR H.264/AAC. Tests cover real CLI/worker processes, the official MCP client,
generated H.264/HEVC/VFR and four rotations, local ASR, backup recovery, and
bounded editing of a one-hour fixture. HDR and nonzero stream starts are
rejected. Real phone footage/appearance and hosted-bot delivery still need
qualification; the full creator product roadmap remains in progress. Read the
[acceptance matrix](docs/implementation-status.md) for precise evidence and limits.

| Document | Purpose |
| --- | --- |
| [Agent guide](AGENT_GUIDE.md) | Requests, source cues, named outputs, revision conflicts and jobs |
| [Deployment](docs/deployment.md) | Native installation, MCP configuration, storage, restart and upgrade |
| [Development](DEVELOPMENT.md) | Latest stable Rust, strict Clippy, generated schema and functional checks |
| [Implementation status](docs/implementation-status.md) | Implemented contracts, acceptance evidence and remaining scope |
| [Product plan](docs/product-plan.md) | Creator workflows and longer-term milestones |
| [Backlog](docs/implementation-backlog.md) | Remaining epics and acceptance criteria |
| [Architecture](docs/architecture.md) | Rust boundaries, exact time and durable state design |
| [Foundation decision](docs/decision.md) | Why the pinned MIT AgentCut core/render libraries were selected |
| [Research](docs/research.md) | Historical candidate source audit and evaluation |
| [Evaluation](evaluation/README.md) | Application checks and the separate candidate harness |
| [Service schema](specs/schemas/service-request.schema.json) | Generated request envelope shared by CLI and MCP |

The application's AgentCut dependency is commit-pinned; its best-effort journal
is not used as project authority. Software here is MIT licensed. FFmpeg, fonts,
models and agent hosts retain their own licenses and operating costs. Never
commit private footage, credentials or model weights.
