# Agent Video Workbench

A Rust video tool for an external agent, available through CLI JSON and MCP
stdio. It owns immutable originals, editable projects, revision history,
analysis, persistent jobs and verified delivery. The agent chooses content and
uses its host's file tools to deliver results.

**[0.5.2](https://github.com/mingley/agent-video-workbench/releases/tag/v0.5.2)** adds ready-to-use preview requests to storyboard tiles, lets
storyboard scopes continue past the 24-hour position, and avoids repeatedly
hashing identical managed originals in the initial render verification pass. It
also resolves cancellation ordering and maps ASR cues to an audited source audio
clock. Older transcript artifacts remain available but may contain shifted times;
new requests use a separate cache. Output duration, byte and integrity checks
remain enforced. Editing and transcript search retain their separate 24-hour
source-position limits.

**[0.5.1](https://github.com/mingley/agent-video-workbench/releases/tag/v0.5.1) improves browsing latency.** Event-driven subprocess completion,
bounded storyboard graphs and one-pass preview validation retain decoded media
and timeline mappings. Five interleaved synthetic runs measured 61% faster
storyboards and 16% faster previews; see [measurements and limits](docs/source-view-performance.md).
Native release qualification is separate from these local measurements.

**0.5.0 adds [source storyboards and budget-aware previews](docs/source-views.md).**
Browse cached thumbnail pages with observed timestamps and transcript context,
then request small clips with measured duration/byte caps and original-frame
mapping. Sparse coverage and unsupported clocks are explicit. The agent selects
content; AVW preserves originals and executes validated operations.

**0.4.0 and later: source-matched cuts and HDR preservation.** Start with
[`assemble` and edit preflight](docs/natural-edit.md) to retain original framing,
color and timing without automatic captions or crop. Plain 10-bit PQ/HLG cuts
export a verified HEVC master and a separate SDR review video. High-quality
masters use CRF 16; lossless video encoding is optional. See the
[editor research](docs/editor-research.md) and [tested scope](docs/implementation-status.md).
For subsequent candidates, [plan edits with source context](docs/edit-planning.md):
briefs, cut reasons, reviewed speech handles, excluded material and playable
source windows help the agent review a paper edit before committing it.
Find literal phrases across transcript cues and listen to source context; map
creator feedback across retained, moved or removed source intervals. Review
pages can copy the exact render and playback time back to the agent.
Check a creator's delivery brief against verified render bytes and draft
chapters from selected markers; both keep the frozen revision visible.
Use the [creator-agent toolkit](docs/creator-agent-toolkit.md) for grounded
highlight proposals, model-visible media, reviewed captions and feedback,
reframing, and checked batch delivery.
Build with Rust **1.99 or newer**: `cargo build --release --locked`.
The checked-in `rust-toolchain.toml` selects stable with rustfmt and Clippy;
the current validated compiler is 1.99.1. Published binaries need no compiler.

The older **0.3.1** Linux binary does not include the new preserving mode. It can import local or
resumable HTTPS originals, convert PQ/HLG HDR to Rec.709 SDR, inspect source
frames/PTS/scenes/silence, transcribe locally, and compose captioned shorts.
Revise with selective source restoration, saved profiles/templates, B-roll,
independent format/language variants, audio ducking/loudness, reversible grades
and editable tracked crops. Freeze a batch and deliver MP4s, SRT/VTT, covers,
contact sheets and a local HTML review bundle. Catalog, retention, verified
backup/restore, relinking and supported-cut OpenTimelineIO interchange are included.

Start with [installation and MCP connection](docs/deployment.md), then run
[the preserving workflow](docs/natural-edit.md) on 0.4.0 or later, or
[the captioned SDR walkthrough](docs/first-edit.md). The
[agent workflow](AGENT_GUIDE.md) covers subsequent edits and recovery. Download checksum-verified binaries from
[releases](https://github.com/mingley/agent-video-workbench/releases).
From a repository checkout, choose a published version listed on the releases
page. The native binary installer needs no Rust compiler, Node, GUI or root
(see deployment for installation without a checkout):

```sh
AVW_VERSION=0.5.2
scripts/install-release.sh /path/to/new-bin "$AVW_VERSION"
scripts/setup-media.sh /path/to/tools
/path/to/new-bin/avw --ffmpeg /path/to/tools/ffmpeg8/linux/ffmpeg \
  --ffprobe /path/to/tools/ffmpeg8/linux/ffprobe doctor
/path/to/new-bin/avw create /path/to/project --name "First short"
/path/to/new-bin/avw agent-guide
```

Building uses Rust 1.99+, rustfmt and strict Clippy. FFmpeg **8.0.1 is the
qualified backend**, with the required caption/HDR/audio filters and libx265 for
10-bit preservation. `doctor` checks capabilities and small encode/decode trials,
not a major-version gate: older builds may pass and work for some media without
being covered by the full qualification matrix. A passing 7.1.5 trial is not a
claim that every workflow is qualified on FFmpeg 7.
Captioned compositions require an imported licensed TTF font.
Optional `scripts/setup-asr.sh PROVIDER_DIRECTORY` prepares pinned whisper.cpp
and checksum-verified English weights. Other analysis providers use an explicit
versioned process contract. No proprietary model account is required.

SQLite commits edits, history and retry outcomes together. Workers freeze input
revisions, fence ownership, bound child memory/files/logs, and support cancellation,
crash recovery and retries. Exports publish only after full decode and technical
QC. Originals and historical references survive cache collection.

The [acceptance matrix](docs/implementation-status.md) records actual generated
media, official MCP SDK, native CI, local ASR, short 4K/60fps and one-hour-source
tests. Compositions deliver SDR H.264/AAC; plain source-preserving edits also
support qualified 10-bit PQ/HLG HEVC. Real phone appearance and additional scripts,
camera profiles, operating systems and hosted agent products need their own
qualification; the tool reports unsupported rendering paths explicitly.

| Document | Purpose |
| --- | --- |
| [Source views](docs/source-views.md) | Cached source pages, observed timestamps and constrained previews |
| [First edit](docs/first-edit.md) | Tested import-to-delivery CLI/MCP walkthrough |
| [Natural edit](docs/natural-edit.md) | Source-matched HDR/SDR baseline, preflight and review |
| [Edit planning](docs/edit-planning.md) | Briefs, source context, speech handles and durable cut decisions |
| [Creator workflows](docs/creator-workflows.md) | Find, edit, compare, deliver and revise content with an agent |
| [Creator-agent toolkit](docs/creator-agent-toolkit.md) | Source evidence, agent media, review, word alignment and batch delivery |
| [Delivery checks and chapters](docs/creator-delivery-checks.md) | Check a creator's constraints and prepare reviewed chapter drafts |
| [Compare edits](docs/compare-edits.md) | Compare frozen source ranges and review unsupported cases |
| [Transcript search](docs/transcript-search.md) | Bounded phrase search across source cues and playable context |
| [Review feedback](docs/review-feedback.md) | Artifact-anchored intervals and mapping across later edits |
| [Caption timing](docs/caption-frame-timing.md) | Review cue boundaries that are shorter than an output frame |
| [Inspection timing](docs/inspection-timing.md) | Interpret bounded scene and silence evidence |
| [Creator opportunities](docs/creator-agent-opportunities.md) | Prioritized creator trials and future product work |
| [Editor research](docs/editor-research.md) | Official editor documentation and applied design choices |
| [Creator delivery research](docs/creator-delivery-research.md) | Research behind discovery, captions, chapters and delivery checks |
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
