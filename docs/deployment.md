# Install and connect an agent

Version 0.2 is a local, single-user Linux tool with a CLI and an MCP stdio
server. The external agent supplies conversation, editorial choices and file
delivery. The workbench persists projects, history, original bytes, analysis,
jobs and verified exports. No web listener or account credentials are required.

## Installation

Build a native bundle with `scripts/package.sh NEW_DIRECTORY`, or use the
prepared bundle in the cloud environment. Verify the adjacent SHA256SUMS,
extract the archive, and run `avw/install.sh NEW_BIN_DIRECTORY`. The installer
verifies the binary and refuses replacement of an existing binary. A versioned
bin directory permits switching back to a previous executable without
modifying it. The qualified binary is Debian 13 x86_64 and dynamically needs
libc, libm and libgcc_s; it is not a universal Linux/macOS/Windows archive.

Set FFmpeg/ffprobe paths explicitly. `scripts/setup-media.sh TOOL_DIRECTORY`
provides the tested checksum-verified FFmpeg 8 route for Linux x86_64. Keep its
FFmpeg license obligations when distributing that separate backend. Import a
licensed TTF font for reproducible captions. Run readiness against the actual
paths:

```sh
/path/to/avw --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe doctor
/path/to/avw capabilities
```

`doctor` performs a real H.264/AAC captioned encode and full decode; check the
JSON `ok` and `ready` values. A process start alone is insufficient. Generated
media qualification on this route is recorded in
[implementation status](implementation-status.md).

## CLI and MCP

Create a directory for projects and input files outside the source checkout.
`avw --workspace ROOT request FILE` runs a typed JSON command; `FILE` may be
`-` for stdin. Requests use camelCase fields and a kebab-case `command`.
`avw schema` returns the shared request schema; `agent-guide` returns the
workflow. Flat CLI commands are also available through `avw --help`.

Use [examples/mcp.json](../examples/mcp.json) as the agent's configuration and
replace its absolute paths. The workspace root must exist. Start
`avw --ffmpeg FFMPEG --ffprobe FFPROBE mcp --root ROOT` through the MCP client.
The server offers one typed `avw` tool and the `avw://guide` resource. Stdout
contains only JSON-RPC; diagnostics use stderr. Paths requested through MCP
must resolve under ROOT, including source imports, backups and artifacts.
This is local filesystem access for a trusted agent. Use the host's normal
process and filesystem isolation for independently managed users.

Suggested first agent task:

> Read agent-guide and capabilities, inspect resume for project `project`,
> review its three named outputs and their artifacts, then make a new named
> short from the supplied source without removing protected source ranges.
> Render it, wait for succeeded, inspect the sheet, and return the verified
> MP4 path with its revision and SHA-256.

On a clean workspace, start with `create` and import the recording and font.
The agent guide describes transcript import, composition, revisions and jobs.
Artifacts are returned as paths and hashes; the host must provide file delivery.
Technical QC cannot choose content or approve transcription and appearance.

## Optional local transcription

Build `scripts/setup-asr.sh PROVIDER_DIRECTORY` with CMake and C/C++. It pins
whisper.cpp v1.9.4 at `927cfce34f31707e17f2bff35c349632fb9e2c3a` and verifies
the public English tiny.en model SHA-256. Whisper and model licenses remain
with their publishers; the application bundle does not redistribute weights.
Keep the build's library directories alongside `build/bin/whisper-cli`.

Before the command or `mcp`, add these global arguments:

```sh
--whisper /path/to/providers/whisper/build/bin/whisper-cli \
--whisper-model /path/to/providers/whisper/models/ggml-tiny.en.bin \
--whisper-model-sha256 921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
```

`transcribe-start` queues a source-audio analysis against an exact revision.
The worker verifies model, program and source hashes, then writes normalized
source cues. Poll the job and obtain `artifact`; inspect its transcript before
`transcript-import`. Analysis does not change project history. Matching source
and provider fingerprints reuse analysis; changing caption style does not
transcribe again. English generated speech is tested; recognition quality on
real voices, other models and languages requires review. Imported reviewed
transcripts work without an ASR provider.

## Persistence and restart

Projects must live on persistent local block storage with working file locks.
SQLite is the authority; keep the entire project directory and never modify
its originals. Revision, request hash/outcome and head commit together with
WAL/FULL sync. JSON exports are snapshots. Same request/key replays its result;
changed intent needs a new key. Use resume/diff after a revision conflict.

`render-start` launches a worker when the host allows detached processes.
For hosts that reap subprocesses at request completion, use `noLaunch: true`
and supervise `avw worker PROJECT --idle-seconds 60` in the host's job runner.
This command drains the queue and exits after the idle timeout; invoke it again
for later queued work. There is one execution owner per project. A restarted
worker acquires its OS lock, marks abandoned attempts interrupted, and requires
explicit job-retry. The logical job ID stays stable and attempts increment.
Cancellation kills the owned process group and leaves previous finals intact.
No live process is assumed to survive cloud snapshot publication.

Import uses a separate owner lock and records copying/verifying/ready/terminal
stages. The next import reconciles abandoned staged files. The committed
request outcome is authoritative if a crash occurred between its transaction
and the final import-stage update. Retry with the original request key and
bytes. Imports do not overwrite the caller's source.

Outputs publish only after full decode, frame/dimension/rate/codec/color/audio
checks and a synced manifest/contact sheet. Only a succeeded job exposes a
verified artifact. Artifact reads recheck its hash. Failed or interrupted
attempt directories can retain diagnostics and scratch; monitor free space
and manage disposable cache/attempt data during maintenance with workers
stopped. There is no automatic retention policy in 0.2. Never prune originals
or the database to reclaim render space.

`backup PROJECT NEW_DIRECTORY` snapshots the database and copies every original
referenced throughout history, verifies bytes and removes the incomplete
marker only on completion. Open the copied directory to restore. Derived
artifacts and analysis caches are omitted; copied jobs are unavailable. Before
upgrading, stop workers and keep a portable backup plus the earlier executable.
Schema 1–3 upgrades retain a pre-migration database copy. Back up before migration
and use a separate restored project with the earlier executable for rollback;
older executables cannot open schema 4.

## Supported limits

The supported delivery path is SDR H.264/AAC MP4, up to one hour per output and
4096 pixels per canvas axis. Render encoding uses two threads and one worker
per project; process logs and analysis requests are bounded. Free-space checks
are estimates, not reserved capacity. The operator controls workspace disk and
the number of projects running in parallel. Local ASR accepts up to one hour
of source audio; inspection analysis accepts at most five minutes per request.

PQ/HLG/BT.2020, detected Dolby Vision and nonzero stream starts are rejected
before an edit commits. Supply an explicitly normalized SDR derivative for
these inputs and preserve the original separately. Real iPhone color/audio,
complex camera modes, scripts/fonts beyond the tested font, hosted-bot storage
and file-delivery behavior remain outside the qualified matrix. The broader
product milestones remain listed in the backlog.
