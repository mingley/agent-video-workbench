# Install and connect an agent

Version 0.3.1 is a local single-user Linux CLI and MCP stdio server. It persists
projects, originals, revisions, analysis, jobs and verified exports. The agent
provides editorial decisions and uses its host's file tools for delivery.

## Published installation

[Releases](https://github.com/mingley/agent-video-workbench/releases) provide
Linux x86_64 and aarch64 archives and a combined SHA256SUMS. Native Ubuntu
x86_64/ARM64 CI qualifies each archive through installed-binary workflows.
Use Ubuntu 24.04 or newer/glibc-compatible Linux; Debian 13 x86_64 is also tested.
The binary dynamically needs libc, libm and libgcc_s. Mac/Windows are not supported.

From the checkout, `scripts/install-release.sh NEW_BIN_DIRECTORY 0.3.1`
downloads the matching archive, verifies its checksum and installs without root.
Without a checkout, download `install-release.sh` and `setup-media.sh` from the
[0.3.1 release](https://github.com/mingley/agent-video-workbench/releases/tag/v0.3.1).
Run them with Bash and the same directory/version arguments; the release
installer needs curl, SHA-256 utilities and tar. For offline installation, transfer the archive and SHA256SUMS, verify the
matching checksum, extract, then run `avw/install.sh NEW_BIN_DIRECTORY`.
The installer checks the binary and refuses overwrite; use versioned directories
for upgrade/rollback. The binary needs no Cargo, Node, Python, GUI or GPU.

Configure FFmpeg/ffprobe explicitly. `scripts/setup-media.sh TOOL_DIRECTORY`
uses a pinned checksum-verified FFmpeg 8 Linux x86_64/ARM64 distribution. That
optional dependency setup uses Bash, curl, unzip and Python 3; a qualified
system FFmpeg is another route. Keep the backend's separate license obligations.
Import a licensed TTF font for repeatable captions; fonts and model weights are
not bundled with the application.

```sh
/path/to/avw --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe doctor
/path/to/avw capabilities
```

`doctor` checks required HDR/audio/caption filters and performs a real H.264/AAC
captioned encode/full decode. Require JSON ok:true and ready:true. Source builds
and packaging checks are in [DEVELOPMENT.md](../DEVELOPMENT.md).

## CLI and MCP

Put projects and input files in persistent storage outside the checkout.
`avw --workspace ROOT request FILE|-` accepts the complete typed JSON schema;
commands use kebab-case and fields camelCase. Flat CLI aliases are in `--help`.
Use `schema`, `agent-guide` and `capabilities` for discovery. Follow
[the first-edit walkthrough](first-edit.md) for import through delivery, and
[the interface reference](agent-api.md) for exact response/retry behavior.

Replace absolute paths in [examples/mcp.json](../examples/mcp.json). The root
must exist. The MCP client launches:

```sh
/path/to/avw --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  mcp --root /path/to/workspace
```

The server exposes one typed `avw` tool and `avw://guide`. Stdout is JSON-RPC;
diagnostics use stderr. Every path, including imports, providers' result
artifacts, backups and delivery destinations, is scoped to the root. This is
trusted local filesystem access; it is not an Internet listener.

Suggested agent trial:

> Read agent-guide and capabilities, then resume the supplied project. Inspect
> original source frames and its source transcripts. Create a new named short,
> preserve protected source ranges, apply the saved profile, and make an
> independent square variant. Freeze both in a batch. Poll until succeeded,
> retrieve verified artifacts and sheets, package delivery, and return its
> MP4s and HTML review bundle with revisions and hashes. Reopen the project
> in a fresh session and selectively restore an omitted clip without undoing style.

On a clean workspace, create a project and import the recording and font first.
The [guide](../AGENT_GUIDE.md) supplies workflow and mutation/retry rules.
Actual CLI/MCP conformance uses the official SDK; other hosts need their own
shell/custom-MCP and file-delivery trial. No remote HTTP MCP transport is included.

## Providers and transfers

Optional local transcription: build `scripts/setup-asr.sh PROVIDER_DIRECTORY`
with CMake/C/C++. It pins whisper.cpp v1.9.4 at
`927cfce34f31707e17f2bff35c349632fb9e2c3a` and verifies public tiny.en weights.
Keep build libraries beside the executable. Global arguments before the command:

```sh
--whisper /path/to/providers/whisper/build/bin/whisper-cli \
--whisper-model /path/to/providers/whisper/models/ggml-tiny.en.bin \
--whisper-model-sha256 921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f \
--whisper-language en
```

`transcribe-start` verifies source/model/program, caches normalized source cues
and leaves editing history unchanged. Review before transcript-import. Model
language compatibility is operator responsibility; English generated speech is
qualified. Imported reviewed transcripts require no provider.

Generic analysis uses optional `--analysis-provider /absolute/executable`.
The executable receives `--avw-request INPUT.json --avw-result OUTPUT.json`.
The input contains schemaVersion:1, kind:analysis, task/parameters and an exact
source path/hash. Output must contain schemaVersion:1, kind:analysis, the same
sourceSha256 and a data object. Its executable SHA enters frozen/cache identity.
It runs with child memory/file/time/output limits, process cancellation and
untrusted-result review. The contract returns analysis rather than project edits.
The executable itself is a trusted operator program with the operator's OS file
permissions; child limits do not create a filesystem sandbox.
Configure any remote data transfer/cost within that provider explicitly.

`--download-host files.example.com` enables direct HTTPS imports for that exact
host; repeat for legitimate redirect destinations. By default none is enabled.
Remote imports additionally need curl and working system TLS trust. URLs may be signed, but embedded username/password are refused. Content length
and byte/free-space bounds are required. Strong ETags permit range resume;
changed identity restarts. Sharing HTML pages fail with a precise error. A supplied
SHA verifies bytes before ingest. Signed URLs do not enter SQLite/portable history.
Use storage connectors to materialize authenticated original files when necessary.
`--download-loopback-http` is only for explicit local development fixtures.

## Persistence, maintenance and rollback

Use persistent local block storage with working OS locks. Keep complete projects;
SQLite is authority and JSON is a derived snapshot. Edits/history/outcomes commit
with WAL/FULL sync. Retry unchanged requests with the same key; reconsider after
revision conflicts. Every historical managed original is immutable.

Auto-launched workers require a host that retains detached processes. Otherwise
enqueue with noLaunch:true and supervise
`avw worker /absolute/path/to/project --idle-seconds 60`. The worker path is
used directly; service-root-relative path resolution does not apply to it.
One owner drains each project's queue. A restarted worker reconciles abandoned
attempts as interrupted; explicit job-retry creates a fresh attempt with the same
logical ID. Cancellation kills the owned process group and retains earlier finals.
Native batch output jobs freeze all revisions atomically and recover independently.
No live process is assumed to survive a cloud snapshot.

Imports and transfers have independent owner locks and managed staging. Query
request-outcome after an uncertain reply, then retry original intent/key/bytes.
Strong-ETag partials survive transfer interruption. Do not manually prune managed
originals or the database to free space.

`cache-gc PROJECT` defaults to dry-run. Enable automatic collection through a
studio retention edit with enabled:true and graceSeconds (minimum 60); it runs
when a worker drains. A daily scheduler can also invoke:

```sh
/path/to/avw maintain /path/to/workspace --grace-seconds 86400 --dry-run false
```

Inspect deferred-project errors; busy worker/import/transfer/inspection leases
are retried on a later invocation. Collection retains all originals/history,
succeeded artifacts, quarantine bytes and recoverable attempts. It removes only
aged reproducible analysis or terminal scratch. Schedule with the host's job
runner/cron; the workbench does not assume a privileged system daemon.

`backup PROJECT NEW_DIRECTORY` copies a consistent database and every historical
original, verifies hashes and seals a manifest. `backup-restore` checks a fresh
copy. Incomplete bundles refuse open. Backups exclude analysis, renders and
review packages; retain delivery separately. Restored derived jobs are unavailable:
queue fresh render/analysis requests with new keys rather than retrying the
excluded artifacts or expecting an interrupted batch to resume.
`verify-project` reports missing/corrupt objects; relink requires the exact hash.
Before upgrading, stop workers, back up and retain the old binary. Schema 1–3
migrations retain a pre-migration database; schema 4 is unchanged in 0.3.1. Roll
back with a separate retained/restored project and compatible old executable.

## Supported limits

Rec.709 SDR H.264/AAC MP4; output up to one hour and 4096 pixels per canvas axis;
one worker per project/two encode threads; per-child 4GiB virtual address space
and 32GiB output file, bounded logs/deadlines. Free-space checks estimate demand;
the operator controls host disk and simultaneous projects. ASR sources are at
most one hour; bounded range inspection, frame-index/proxy/tracking analysis
accept at most five-minute intervals.

PQ/HLG sources retain original HDR bytes and receive an explicit recorded tone
map before SDR composition. Nonzero starts, audio gaps and SAR/orientation have
synthetic qualification. Dolby Vision compatible profile 8 base layers are
recognized; profile 5 has no delivery path. HDR output and unqualified wide/Log
transforms have no supported delivery path.
Other camera modes/scripts/OSs need implementation or dedicated qualification;
real phone appearance needs creator review. Read
[status](implementation-status.md) for the exact evidence.
