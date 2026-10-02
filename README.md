# Agent Video Workbench

An evidence-backed plan for an open-source Rust video editor operated by an
external persistent agent. The first user is an iPhone content creator who
wants to send footage, review drafts, and request revisions conversationally.

**Status: Rust prototype and candidate evaluation, October 2, 2026.** The
[development guide](DEVELOPMENT.md) describes the transactional project store
and synthetic SDR editing/render loop. Creator acceptance is still pending. `agent-video-workbench` is a provisional
repository name; a product name remains open. Reelwright is already used by
other video products and a GitHub project.

**Recommended starting point:** extend the MIT-licensed AgentCut Rust core and
renderer in a focused application, with durable transactional project history,
iPhone ingest, transcription, and a small agent interface. Pin the upstream
revision. Address the reproduced rotation failure and history loss before
calling it suitable for this workflow. Keep OpenReelio as the alternative.

The external bot owns conversation, reasoning, and its own memory. This tool
owns media, editable projects, source references, revisions, jobs, and render
evidence. FFmpeg supplies media processing; a GUI is unnecessary.

| Document | What it answers |
| --- | --- |
| [Decision and MVP](docs/decision.md) | Adopt, extend, fork, or build; one complete creator workflow |
| [Candidate research](docs/research.md) | Pinned source audit, licenses, installs, verified edits, limitations |
| [Architecture](docs/architecture.md) | Rust boundaries, durable state, exact timing, inspection, rendering, recovery |
| [Implementation backlog](docs/roadmap.md) | Ordered phases with acceptance criteria and first prototype |
| [Hosted-bot compatibility](docs/host-compatibility.md) | Dots, Grok Bot, Muse, installation, transfer, storage, GPU assumptions |
| [Evaluation](evaluation/README.md) | Reproduce the tests and inspect generated artifacts |

Built and ran AgentCut, ave, and OpenReelio from pinned source on Apple Silicon.
The synthetic edit retained two ranges, removed a four-second pause, burned
captions, and rendered a draft. AgentCut also restored an omitted interval in
one batch, changed captions, and undid/redid the revision across CLI processes.
These checks do not establish real iPhone, HDR, transcription, long-video, or
hosted-bot compatibility.

![Generated draft contact sheet](evaluation/results/draft-sheet.png)

Software in this repository is MIT licensed. External agents, optional hosted
transcription, storage, bandwidth, and compute can cost money. FFmpeg builds,
models, and fonts retain their own licenses. No proprietary model service is
required by the planned editor.
