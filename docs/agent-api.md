# CLI and MCP interface reference

The 0.4.0 source build's CLI and MCP stdio server call the same Rust service.
Published 0.3.1 lacks `assemble`, `edit-preflight` and preview retrieval.
[The first-edit walkthrough](first-edit.md) contains an executable workflow;
[service-request.schema.json](../specs/schemas/service-request.schema.json)
is the generated request contract. `avw schema` returns it in `result`.

## Requests and responses

Requests use a kebab-case `command` and camelCase fields. Supply the entire
envelope to `avw request FILE`, `avw request -`, or the MCP `avw` tool:

```json
{"command":"resume","project":"project"}
```

CLI aliases have their own arguments: `avw resume project` issues that same
request. `--help` lists available aliases. Operations such as `import-url` and
`analyze-start` use the JSON route. `compose --request FILE` and
`assemble --request FILE` aliases expect
only the edit object, whereas `request FILE` expects the complete envelope.
Use [current examples](../examples/requests/README.md) for the latter.

A successful service response has this shape:

```json
{"ok":true,"apiVersion":"1","result":{}}
```

A service failure has `ok: false` and `error` with `code`, `message`, `details`
and `retryable`. Inspect the result for each command; there is no universal
`requestId`, `warnings`, `phase` or progress percentage. CLI argument errors can
have a smaller error object. Help/version output is text. MCP stdout carries
JSON-RPC; inspect the tool's response content and error indication, not only
transport success. Job creation acknowledges a queue entry, not a final file.

Service envelopes reject unknown fields; the parser also rejects duplicate
object keys and integers outside
the exact JSON range. Schema validation checks structure; application validation
also checks IDs, time arithmetic, source hashes, policy and current revision.
The [older protocol examples](../examples/protocol/README.md) illustrate a
design draft and are not current wire responses.

## Revisions and safe retries

Creator editing mutations require `expectedRevision` and `key`. Raw `apply`
instead embeds an AgentCut `batch` with `baseRevision` and `idempotencyKey`,
plus its schema/project IDs and operations. Keys are 8–200 bytes;
reuse a key only for exactly the same intent. Same-key/same-intent replay returns
the persisted outcome even when the head has advanced. Changed intent needs a
new key. A revision conflict reports `expectedRevision` and `currentRevision`:
read `resume`/`diff`, reconsider the edit, then submit against the current head.

`dryRun: true` validates without committing history or a success outcome.
`request-outcome` checks an uncertain committed reply. Imports, transcripts,
assemble, compose, studio edits and raw apply batches advance the editing head. Inspection,
analysis and rendering do not. Render and batch creation still check the supplied
revision and persist idempotent queue outcomes.

Use stable IDs containing 1–80 ASCII letters, digits, `_` or `-`; keep asset,
sequence and item IDs distinct. `compose` uses source `startMs`/`endMs` with
half-open intervals and creates a 30 fps sequence. `assemble` matches upright
source dimensions/rate, preserves qualified color by default, adds no captions,
and retains exact requested source starts with frame-rounded durations. See
[natural edits](natural-edit.md) for flags, color choices and explicit VFR
conformance. `edit-preflight` is read-only and reports actual decisions; it
does not approve the content or guarantee every composition is renderable.
Raw operations use the
exact rational time forms described by `describe OPERATION`. A property in
that registry is not a guarantee that every renderer combination is supported.

## Find state without resending the project

| Request | Result / intended use |
| --- | --- |
| `resume` | Compact `projectId`, `name`, `revision`, outputs, assets, `protectedRanges`, recent history and jobs |
| `status` | Full project snapshot with assets, sequences and extensions; protection is not a top-level `protectedRanges` field |
| `history` | Revision-bounded page with `items` and `nextAfter` |
| `studio-state` | Prefix-filtered decisions with `nextOffset`; large values are summarized |
| `transcript-search` | Matching source cues with `nextOffset` |
| `catalog` | Read-only workspace discovery with `nextOffset` |
| `diff` | Changes between explicit revisions |
| `jobs`, `artifacts`, `imports` | Operational state; these are not interchangeable with editing history |

Follow returned pagination markers rather than assuming the first page is
complete. Read schema defaults and limits for each operation. `resume` includes
at most 100 assets and reports truncation; use narrower discovery as needed.

## Jobs and verified delivery

`render-start`, `transcribe-start` and `analyze-start` return a durable job `id`.
`batch-start` returns a parent ID and freezes all selected outputs atomically.
Use `job-status` for children and `batch-status` for aggregate completion.
The parent is a `batch` record; aggregate status is computed from its children.

Runnable jobs move from `queued` to `running`, can enter `verifying`, then
finish as `succeeded`, `failed`, `cancelled` or `interrupted`. Continue polling
through `verifying`; encoding completion alone does not mean a deliverable is
ready. Short stages may finish between polls. Restored backups mark
derived jobs `unavailable`. `job-retry` accepts failed/cancelled/interrupted jobs
and starts a fresh attempt with the same logical ID. It cannot resume an encode
from its last frame or recover excluded backup artifacts.

`noLaunch: true` leaves jobs for a foreground, supervised worker. Invoke
`avw worker /absolute/path/to/project`; the worker's project argument is a
filesystem path, not a service-root-relative locator. Priority is -100..100,
larger first. Later edits never change a queued job's frozen revision.

`artifact` rechecks bytes and returns `path`, `mimeType`, `bytes`, `sha256`,
`revision` and `verified`. Render results also include the manifest path;
analysis results have task-specific data/attachments. `sheet: true` verifies
the render contact sheet. `preview: true` returns the hash-verified SDR review
video when a separate one exists; otherwise it returns the SDR master. A
`role` identifies `master`, `sdr-review-preview` or `contact-sheet`. Combining
`sheet: true` with `preview: true` refuses. Analysis artifacts do not support
video-preview retrieval.
The synchronous `render` alias returns `artifactId`, `path`, `manifest` and
`revision`; retrieve `artifact` with that ID for the verification/hash response.
`delivery` packages a succeeded render or batch into a new scoped directory.
Read its manifest for failures and unresolved feedback, then use host file tools
to return the files. See [support limits](implementation-status.md#explicit-support-boundaries).

## Filesystem and process boundary

With `--workspace ROOT`, service paths resolve inside an existing root. MCP
always has a root. Without a CLI root, paths are ordinary local paths. Keep
projects and inputs on persistent local storage. The CLI reads request files
itself; they can reside in a checkout outside the media workspace.

This is a trusted local, single-user process. There is no HTTP listener, account
authentication or filesystem sandbox for configured provider executables.
Providers receive source paths and run with the operator's OS permissions,
plus child resource/time limits. Configure only trusted programs. Direct HTTPS
imports require explicit allowed hosts. See [deployment](deployment.md) for
providers, transfers, persistence and maintenance.
