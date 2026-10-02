# Distribution, workers and hosted operation

Status: proposed. Features: F16, F18, F19 and F22. Provider capabilities remain
those in the dated [host compatibility research](../host-compatibility.md);
this specification does not establish new access or quota claims.

## Supported execution routes

| Route | Prerequisites checked on the actual account | Execution |
| --- | --- | --- |
| Installed CLI | Executable downloads, supported OS/CPU, writable persistent files, sufficient process lifetime | Agent invokes `avw`; Rust executes media jobs locally |
| Local MCP | Host can launch a custom stdio server and access project files | Thin MCP adapter calls the same application service |
| Remote worker | Bot can connect to an authenticated custom tool route and transfer authorized media | Rust service owns project storage, jobs and artifact access |

A hosted bot with none of these routes is unsupported until an adapter becomes
available. An agent having web browsing or file attachments alone is insufficient.
Compatibility reports name product/account mode, date, route, OS/architecture,
durable path, limits actually observed, tests passed and unknown properties.

The first supported route must demonstrate installation, original-file access,
captioned preview, returning the artifact, and reopen after conversation reset.
A second product reuses the protocol and fixture; it is not assumed compatible
because the first product passed. GPU availability is optional.

## Installation and update contract

Publish versioned application archives with checksums, a release manifest,
dependency notices and machine-readable platform/backend requirements. Target
Linux x86_64 and aarch64 first; qualify macOS and Windows archives separately.
Declare required system-library/CPU baselines and test on clean images at those
baselines. `cargo install` can remain a developer route; ordinary agents should
not need Cargo, Python, Node, sudo, Docker or a GUI.

An installer resolves the platform, downloads to a temporary location, verifies
the release hash, runs `doctor` plus a tiny generated encode, and activates a
versioned user-writable tools directory atomically. An existing working version
is retained until the new one passes. Installation produces JSON describing
paths, versions, test results and any missing capability. A repeat installation
can repair removed tools without touching project data.

Select a tested FFmpeg bundle or explicitly validate a system FFmpeg. Check
filters, fonts, decoders and an actual encoder invocation. Include distribution
licenses/build flags; a presence-only `ffmpeg --version` check is insufficient.
Fonts, ASR models and optional providers have separate hashes, versions, license
metadata and download sizes. Offline installation can use a prepared bundle.

Tool updates never migrate a project as a side effect of installation. Opening
an incompatible schema returns a version error and the supported upgrade path.
Explicit migration uses the backup/recovery policy in
[project lifecycle](project-lifecycle.md). Do not promise binary rollback after
an irreversible schema change without a corresponding project backup.

Acceptance: install, update and repair on a clean supported CPU host; render a
caption with the packaged/validated toolchain; reopen an existing project with
its state intact. Removed package installations can be rebuilt from the manifest.

## Worker ownership and jobs

Long jobs require an execution owner with sufficient lifetime. `render start`
enqueues and returns a job ID; it does not imply that a subprocess will survive
the CLI or host sandbox ending. `doctor` reports whether a local worker is
running/launchable, a synchronous mode is usable, or a remote route is required.
Queued work without an available worker has an explicit waiting reason.

The same Rust binary may expose `worker run` and a synchronous `--wait` mode.
A local detached worker is allowed only where the host's lifecycle permits it.
If a host terminates all processes after a tool call, use its supported long-job
facility or the remote worker. Repeated status polling cannot keep an otherwise
unsupported process alive by assumption.

Jobs contain immutable inputs/plan hash, priority, resource estimate, persisted
state, attempts, lease owner, heartbeat and monotonically increasing claim
generation. Claiming is atomic. A new generation fences an expired worker so
it cannot later publish success for the replaced attempt. Attempt paths are
unique; immutable artifact paths prevent a late worker from overwriting a good
output. Only the current generation can commit the final artifact reference.

State transitions are `queued → running → verifying → succeeded`, with explicit
`failed`, `cancelled` and `interrupted` outcomes. A retry adds an attempt to the
same logical job. Completed compatible stages may be reused; incomplete encoding
normally restarts. Cancellation signals the owned process group, escalates after
a bounded grace period, and marks cancellation only when execution has stopped
or its lease is safely fenced. Prior outputs stay available.

Before running, reserve/check scratch space and configured CPU/RAM/GPU budgets.
Limit concurrency by resource class. Short interactive previews should be
scheduled ahead of new batch encodes; initially let running encodes finish or
cancel them explicitly rather than pretending arbitrary mid-encode preemption
is cheap. A single-user queue comes first. Multi-tenant fairness requires another
design once there is an actual shared service requirement.

Acceptance: start competing claims, expire one, and prove that its late result
cannot replace the current attempt. Kill/restart a worker, cancel an encode,
fill scratch storage, and recover without corrupting project state or earlier
finals. Partial batches report each item's outcome.

## Remote worker and extensions

Start with one creator's authenticated worker. The worker owns a local SQLite
database and durable volume; clients never open its database over a network
filesystem. Media blobs/backups may use object storage. Requests and artifact
handles are scoped to the configured workspace/project. Temporary download links
expire independently of durable asset identity. Tool transport does not require
embedding a new conversational agent.

Remote tools provide the same capabilities, typed edits, job polling, cancellation
and artifact manifests as the CLI. Authentication and supported transport must
be integrated with the actual bot. Persist actor attribution from authenticated
context where available; a free-text author field is descriptive metadata, not
an authorization credential.

Optional analysis providers run through a versioned process adapter with input
manifests, declared capabilities, resource/cost settings and structured results.
Timeouts, cancellation, malformed output and unavailable model weights become
job errors. Providers receive only the media/ranges required for their job.
Remote data transfer is an explicit configured provider choice. Do not load
arbitrary native extensions into the project-store process in the first design.

Presets/templates use validated data, not executable code. A provider result
cannot mutate project state directly: attaching analysis or accepting a proposed
edit goes through the normal revision transaction. This keeps provider failures
and future integrations from bypassing the project's history guarantees.

Costs are reported by category: local resource measurements, worker hosting,
storage/egress, and model/provider usage. Dollar estimates require configured
rates and are marked estimates; unknown provider billing stays unknown. Enforce
configured duration/token/byte/compute limits at the job boundary where those
limits are observable. A budget failure leaves existing projects and results
accessible.
