# CLI and MCP interface reference

The 0.4.0 source build's CLI and MCP stdio server call the same Rust service.
Published 0.3.1 lacks `assemble`, `plan-edit`, `apply-edit-plan`, `edit-preflight`
and preview retrieval.
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

Updated source builds add CLI-only `--result-pointer POINTER`, relative to the
successful `result` object. For example, `--result-pointer /id batch-start ...`
prints the batch ID without quotes, and `/items/0/id` selects a child ID from
`batch-status`. Non-string values remain JSON. Errors retain envelopes and
nonzero exit status. Invalid pointer syntax is rejected before execution; a
missing field returns `E_RESULT_POINTER` **after** the operation has succeeded,
including its full result. Do not retry that mutation with a new key. This
presentation option is not supported on MCP stdio and does not change the
service schema. Published 0.4.0 predates the option; use a JSON parser there.

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

`plan-edit` accepts `plan: {edit, brief, decisions}` and `expectedRevision`,
performs no mutation and returns a revision-bound `planSha256`, `readyToApply`,
source contexts, omitted ranges and per-cut risk findings. `apply-edit-plan`
adds that SHA, `key` and optional `dryRun`; it refuses mismatched plans or
unreviewed risks and commits intent with the sequence atomically. Preflight's
`editPlan` reports current/stale/unplanned review state. See
[edit planning](edit-planning.md) for fields and executable CLI/MCP steps.

## Find state without resending the project

| Request | Result / intended use |
| --- | --- |
| `resume` | Compact `projectId`, `name`, `revision`, outputs, assets, `protectedRanges`, recent history and jobs |
| `status` | Full project snapshot with assets, sequences and extensions; protection is not a top-level `protectedRanges` field |
| `history` | Revision-bounded page with `items` and `nextAfter` |
| `studio-state` | Prefix-filtered decisions with `nextOffset`; large values are summarized |
| `transcript-search` | Matching source cues, or opt-in literal matches across adjacent cues, with `nextOffset` |
| `source-evidence`, `prepare-highlights` | Revision-pinned source windows and reviewable, externally proposed clip plans |
| `provider-search`, `provider-search-results` | Explicitly queue and validate configured provider moment-search proposals |
| `agent-media` | Bounded verified frame, sheet or proxy-audio content as base64 JSON |
| `preview-word-alignment`, `accept-word-alignment`, `word-alignment` | Validate, review and retrieve caller-supplied source-word timing |
| `translation-context`, `plan-translation` | Review supplied caption translations and prepare a provenance-bound variant |
| `suggest-reframe` | Review an editable crop path from caller-supplied subject observations |
| `audio-cleanup` | Explicitly set or remove reversible spectral noise reduction on a composed audio item |
| `review-import` | Atomically import a creator's frozen-artifact notes or explicit version decision |
| `batch-delivery-check`, `batch-delivery` | Check and package frozen outputs independently with caller-supplied briefs |
| `thumbnail-candidates` | Extract creator-selected, timestamp-verified images from a frozen review video |
| `host-readiness` | Check local workspace and media prerequisites while reporting host-specific unknowns |
| `compare-outputs` | Exact source-range comparison of two verified queued plain-cut renders |
| `delivery-check` | Check a caller's brief against a frozen render's verified bytes and freshly probed properties |
| `chapter-draft` | Prepare reviewed public chapter text from selected frozen sequence markers |
| `catalog` | Read-only workspace discovery with `nextOffset` |
| `diff` | Changes between explicit revisions |
| `jobs`, `artifacts`, `imports` | Operational state; these are not interchangeable with editing history |

Follow returned pagination markers rather than assuming the first page is
complete. Read schema defaults and limits for each operation. `resume` includes
at most 100 assets and reports truncation; use narrower discovery as needed.
For cross-cue matching, revision-pinned pagination and source-preview requests,
see [transcript search](transcript-search.md).
For comparison scope, artifact integrity and unsupported cases, see
[compare edits](compare-edits.md).
For delivery-brief fields, evidence, chapter selection and limitations, see
[delivery checks and chapters](creator-delivery-checks.md).
The [creator-agent toolkit](creator-agent-toolkit.md) describes the newer
workflows, including optional provider disclosure and model-visible evidence.
The separate MCP `avw-media` tool exposes the same checked media as native
image/audio blocks; the agent host determines whether it can consume them.
For scene and silence inspection, check `analysisStatus`, `unavailableReason`
and coverage: empty evidence is not proof of absence. Long-GOP or sparse seeks,
missing timestamps and discontinuous audio can make analysis unavailable. Stream
edges are not fully measured; scene scores may cross video gaps, and silence
ends may be decoded-stream EOF. See [inspection timing](inspection-timing.md).

## Jobs and verified delivery

`render-start`, `transcribe-start` and `analyze-start` return a durable job `id`.
`batch-start` returns a parent ID and freezes all selected outputs atomically.
Use `job-status` for children and `batch-status` for aggregate completion.
The parent is a `batch` record; aggregate status is computed from its children.
Invalid analysis tasks are rejected against the frozen revision before a job or
request key is recorded. Generated context-preview requests are bound to their
project, source and media configuration; regenerate them on another service
configuration instead of reusing their key.

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
Its legacy manifests include the full frozen project snapshot and current
review history, which can contain unrelated private material; inspect before
sharing. The checked `batch-delivery` workflow provides a reduced manifest for
sharing and reports each output's state separately.
`delivery-check` reports pass/fail/unknown per requested constraint for one
completed queued render; it does not automatically attach the report to that
package or prove upload acceptance. `chapter-draft` reports exact selected
marker times and copyable, whole-second text while leaving eligibility unknown.
Read its manifest for failures and unresolved feedback, then use host file tools
to return the files. The review page can copy the frozen render ID, revision and
current playback time for a creator to send back to their agent. See
[feedback mapping](review-feedback.md) and
[support limits](implementation-status.md#explicit-support-boundaries).

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
