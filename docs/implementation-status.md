# Current implementation evidence

The 0.2 service adds shared typed JSON requests and MCP stdio, with generated
request schemas, bounded resume/history/search, semantic ID diffs and durable
request-outcome lookup. `compose` expands ordered source cuts and imported
transcript cues into independent named outputs in the revision transaction.
Source frames, metadata, silence and scene evidence are bounded and cached.
Managed assets are imported, not injected through raw editing operations.

`render-start`, `worker`, `job-status`, `job-cancel` and `job-retry` use schema 3
frozen inputs, attempts, generations, cancellation and heartbeat. An OS owner
lock permits one worker per project. Reacquisition marks abandoned attempts
interrupted; retry writes a new immutable attempt. Start idempotency prevents
duplicate jobs. The Linux runner bounds logs, imposes deadlines, kills owned
process groups and stops FFmpeg when its worker dies.

Evidence: 15 Rust tests; the application media regression;
`evaluation/job_smoke.py` through actual CLI/worker processes with SIGKILL,
reopen/retry and bounded cancellation; `evaluation/agent_smoke.mjs` through the
official MCP SDK, covering import/transcript/search/compose, cached frames,
background render/QC/artifact, backup and equivalent CLI/MCP replay/conflicts.
Real phone/host qualification remains open.

The inventory below records the preceding prototype baseline; it will be
replaced with the service acceptance matrix once agent integration is validated.

# Implementation status

Source baseline: commit `21cba27`, October 2, 2026. This inventory describes code
present in the repository. It distinguishes that prototype from the broader
[product plan](product-plan.md) and the separately executed
[candidate evaluation](research.md). [DEVELOPMENT.md](../DEVELOPMENT.md) records
the application smoke workflow and its verified synthetic Linux scope.

## Present in the Rust prototype

The `avw` binary exposes project/edit commands plus `doctor`, `capabilities`,
`describe`, `import`, `render`, `jobs`, `protect` and `backup`. It uses the pinned
AgentCut core/render dependencies, SQLite and external FFmpeg/ffprobe.

| Area | Current source behavior | Remaining work |
| --- | --- | --- |
| Project creation | Creates a new directory, media subdirectories and schema-version-2 database; default canvas 360×640 at 30 fps | Configurable format; recoverable interrupted initialization |
| Apply | Reads an AgentCut `OperationBatch`; applies model operations and source-coverage validation inside an immediate SQLite write transaction | Richer result IDs/diffs and application workflow operations |
| Revision persistence | Stores snapshots, parent, request, head and retry outcome transactionally; WAL/FULL sync; schema 1→2 migration retains a backup | Broader crash/storage-fault and migration coverage; detailed provenance |
| Retry/conflict | SHA-256 request hash; same key replays outcome; mismatched key or stale revision errors | Request-outcome query; documented canonicalization; enriched recovery metadata |
| Restore | Copies a prior snapshot into a fresh revision while preserving current protection annotations | Source-based selective restoration and user-facing undo/redo navigation |
| Status/history/diff | Returns current project, full history, or the two requested snapshots | Compact resume status, pagination and semantic diff |
| JSON/discovery | `ok`, `apiVersion: "1"`, `result` or `error`; argument errors use JSON; exposes pinned capability registry and descriptions | Request IDs, complete schema API, host-qualified capability results, recovery details and MCP |
| Managed import | Copies/hashes local bytes into immutable managed originals, probes metadata and rejects detected HDR paths | Remote/resumable transfer, recorded import stages, broad iPhone/HEVC/VFR/HDR qualification |
| Render/QC | Synchronous H.264/AAC render; explicit autorotation handling; decode/frame/dimension/rate/tag/audio checks, contact sheet and revision manifest | Async execution, richer presets, color-managed phone qualification, caption/semantic quality checks |
| Protection | Enforces exact source-time coverage in enabled video clips across edits/restores; cannot remove protection implicitly | Explicit policy editing, stream/speed policies and visibility/crop review; no unprotect command yet |
| Jobs | Persists running/succeeded/failed render records tied to a revision | Leases, cancellation, start idempotency, automatic crash reconciliation and batch scheduling |
| Backup | CLI creates a consistent DB snapshot plus originals referenced throughout history; incomplete copies refuse to open; omitted render records become unavailable | Rich bundle manifests, archive transport, relink workflow and wider recovery tests |
| Packaging | Native archive/installer scripts with licenses, dependency declarations and checksums; FFmpeg/fonts/models remain external | Published releases, qualified platform/host matrix, managed updates and offline dependency bundles |

`--help` and `--version` use normal human-readable CLI output. The application
does not yet expose transcription, asynchronous persistent workers, style
libraries or MCP. Rendering is SDR-only; HDR rejection and Rec.709 output tags
do not establish complete color-managed iPhone support. The upstream capability
registry is broader than the application's acceptance matrix.

The source includes tests for reopen/retry/restore, invalid batches, dry-run,
concurrent writers and rollback when request insertion is rejected by a SQLite
trigger, plus source-protection and migration/backup tests. The application smoke
harness covers the generated SDR edit/revise/render/backup loop documented in
DEVELOPMENT.md. The SQLite trigger is a transaction-failure simulation, not an
actual disk-full or power-loss experiment. This specification expansion does
not rerun or broaden that media/host evidence.

## How to interpret the specifications

The original [architecture](architecture.md) remains the broad design. The
[specifications](specs/agent-protocol.md) make behavior and acceptance gates more
concrete. Proposed commands and operation extensions are explicitly labeled.
The current CLI keeps its flat commands and AgentCut batch format until an
intentional compatibility change implements any new surface.

The [draft schemas](../specs/schemas/README.md) validate example wire structure;
they are not generated from current Rust types and do not certify domain/media
validity. Before shipping them as the runtime contract, add conformance checks
against Rust parsing, responses and capability-derived operation schemas.

No downloadable application release or supported-host certification is implied
by the source package's `0.1.0` version. Published compatibility claims require
the [deployment](specs/distribution-and-workers.md) and
[quality](specs/quality-and-performance.md) gates.
