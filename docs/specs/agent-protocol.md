# Agent interface and operation contract

Status: design specification for F18, with F01/F03/F16 integration. Many
capabilities are implemented in 0.3.1, but the proposed command families and
response metadata below are not the runtime wire reference. Use
[the current interface](../agent-api.md),
[generated schema](../../specs/schemas/service-request.schema.json) and
[current request examples](../../examples/requests/README.md) for integration.
[Implementation status](../implementation-status.md) records tested behavior.
Draft schemas validate structure rather than operation semantics.

## Discovery and bounded interaction

The proposed `capabilities` response describes protocol/schema versions,
implemented operations, supported media paths, limits and unavailable features
with reasons. `schema` returns the envelope plus operation/property schemas;
`agent-guide` supplies short examples and recovery guidance. `doctor` adds
host-specific evidence: tool/filter versions, writable project path, actual
encoder test, worker availability and storage checks.

Expose a small top-level summary and queryable capability groups. Avoid sending
all effects, transcripts or project snapshots by default. Unsupported operations
are rejected explicitly; neither the agent nor an MCP adapter may infer support
from a feature existing in this product plan.

| Command family (proposed) | Result contract |
| --- | --- |
| `project create/open/status` | Project identity, revision, format and compact resume context |
| `asset import/list/relink`, `inspect` | Asset handles, supported processing paths, job or analysis references |
| `transcript search`, `annotation list/protect` | Bounded source-time evidence and stable annotation IDs |
| `edit validate/apply`, `history`, `diff`, `restore` | Normalized changes, revision outcomes, explicit restoration semantics |
| `variant create/compare`, `profile list/apply` | Explicit versions, overrides and affected sequences |
| `preview`, `render start`, `job status/cancel` | Immutable inputs, job handle, state and artifact references |
| `artifact list/get`, `verify`, `review add/resolve` | Exact revision/hash provenance and technical/editorial status |
| `project bundle/restore`, `cache inspect/collect` | Included/missing objects, dependencies and retention effects |

Retain existing flat CLI commands as aliases while introducing command families,
or document an intentional breaking release. Human `--help`/`--version` output
is allowed. Data commands emit one JSON envelope on stdout; logs and NDJSON
progress go to stderr. A killed process may emit no response, so durability and
request-outcome lookup are necessary regardless of the output contract.

## Atomic edit requests

Requests use `schemaVersion`, `projectId`, `baseRevision`, `idempotencyKey`,
optional `author`/`description`, and an ordered `operations` array. Each operation
has `id`, `op`, optional `target`, and `params`. The operation registry validates
`params` against the selected operation/property type before domain validation.
Application conveniences such as source-based restoration expand into typed
core operations plus editorial metadata inside one transaction.

Stable object IDs come from prior results or explicit caller-provided creation
IDs. Later operations in a batch may reference earlier created IDs. Validate
that dependency order and reject duplicate operation IDs. Do not use timeline
array indexes as persistent references. A human-friendly name is searchable
metadata, not a unique project/clip identifier.

Use exact rational time objects compatible with the core: integer `value` plus
`rate.numerator` / `rate.denominator`, expressing `value / rate` seconds. Source
time is interpreted through the selected stream's timebase/PTS mapping; output
time through the sequence rate. Intervals are half-open. Cross-rate conversion
uses checked rational arithmetic with declared rounding at output boundaries.
Negative stream timestamps can exist; negative output positions are rejected
unless a specific operation supports them. The public JSON profile restricts
integers to the exact interoperable range ±(2^53−1), rejecting overflow rather
than truncating. Wider internal time values require an intentional wire-format
extension before exposure.

An atomic batch follows this sequence:

1. Parse/version-check the request, reject duplicate JSON keys and unknown
   request fields, resolve operation schemas and normalize documented defaults.
2. Begin the write transaction; check the idempotency key before the base revision.
3. Validate the expected project/revision and apply operations to a candidate.
4. Validate source ranges, A/V links, font/caption bindings, protection policies
   and supported feature settings. Media execution happens outside this lock.
5. Commit snapshot, history, created/changed IDs, omission records and the exact
   durable outcome together; only then return success.

Same key plus the same normalized request returns the recorded outcome, even
if head has advanced. Same key plus a different request returns
`E_IDEMPOTENCY_CONFLICT`. Scope keys to a project. Canonicalization sorts object
keys recursively, preserves array order, includes intent/author fields and uses
documented JSON number encoding after typed normalization. Trace/request IDs
generated for an invocation do not change edit identity. Hash schema/normalizer
version with the canonical body. Preserve old outcome/hash versions across
upgrades so a later retry remains recognizable.

Dry-run returns base revision, predicted new revision, normalized operations,
changed/created IDs, duration/source coverage, warnings and required jobs. It
writes no successful request outcome and reserves no revision. If it refers to
an already committed key, report that prior outcome explicitly rather than
presenting the edit as newly validated. Failed validation leaves head and
history unchanged. A retry after an unknown outcome first queries by key.

The [example request](../../examples/protocol/edit-request.json) uses existing
core operation names against an illustrative revision containing a caption.
It is not executable against an empty project without replacing its IDs/state.

## Responses, errors and pagination

Success: `ok`, `apiVersion`, `result`, plus proposed `requestId`, command and
warnings. Edit results include project ID, previous/new revision and operation
results; replay information is explicit. Failure: `ok: false`, `apiVersion`,
`error.code` and `error.message`, with proposed phase, affected ID/field path,
recoverability and structured details. Clients tolerate new optional response
fields; they must not ignore unknown required request semantics.

| Error family | Recovery instruction |
| --- | --- |
| `E_ARGUMENT`, `E_JSON`, `E_INVALID_REQUEST` | Repair syntax/schema; do not blindly retry |
| `E_CAPABILITY_UNAVAILABLE` | Inspect availability and choose a supported operation/backend |
| `E_REVISION_CONFLICT` | Read current head/diff, reconsider the edit and use a new key for a changed request |
| `E_IDEMPOTENCY_CONFLICT` | Query the original result; never reuse that key for different intent |
| `E_SOURCE_MISSING`, `E_SOURCE_CHANGED` | Relink/restore by expected content identity |
| `E_REFERENCE_UNRESOLVED`, `E_PROTECTED_RANGE` | Inspect referenced source/edit and resolve the proposed change explicitly |
| `E_FONT_MISSING`, `E_MODEL_UNAVAILABLE` | Install the pinned dependency or select an explicit supported alternative |
| `E_STORAGE_CAPACITY`, `E_WORKER_UNAVAILABLE` | Repair capacity/execution route, then retry the job/stage |
| `E_RENDER_FAILED`, `E_QC_FAILED` | Inspect bounded diagnostics and the render plan/artifact; preserve the earlier final |
| `E_SCHEMA_UNSUPPORTED` | Use a compatible binary or the supported migration path |

Retain existing prototype error codes while adding more precise structured
details. Proposed process exit categories are success 0, domain/execution error
1 and argument/parse error 2, matching the current broad convention; the JSON
code is the authoritative machine distinction. MCP wraps the same application
error data in its tool result instead of silently returning a successful edit.

Lists/search/history have a bounded `limit` and opaque `cursor`, an explicit
snapshot revision or analysis version, and `nextCursor`/completion status.
A cursor that cannot retain its snapshot returns an expiry/conflict error;
pagination must not silently mix revisions. Large text/media is an artifact
handle or paginated resource, not an enormous inline tool response.

## Resume, jobs and artifact access

`project status` provides a bounded resume packet: head revision, named outputs,
saved profile versions, source availability, protected ranges, recent changes,
pending/failed jobs, unresolved reviews and artifact references. Include hashes
and revision IDs so the agent can identify stale previews. The agent may then
query the relevant sequence, omission or transcript range. A full project export
is available separately.

Long work returns a job ID with state and the captured revision/plan hash. Job
status exposes stages and attempts; encoding reaching 100% does not mean output
verification succeeded. Persist job start idempotency independently of edit
keys so a lost response cannot queue duplicate renders. Cancellation and retry
also report their actual durable state.

Artifact handles resolve only within the authorized project/workspace. A
download response declares MIME type, byte length, hash, revision, technical
status and expiry if access is temporary. Do not embed video bytes in JSON.
The same logical artifact works through a local path or authorized remote
download; external locations/credentials are transport details.

MCP is a thin adapter over this service: discovery, project resources, typed
operations, jobs and artifacts. Start with stdio and qualify a remote transport
only for a real host route. It uses the same validation, expected-revision checks
and idempotency outcomes as the CLI. No operation is more permissive merely
because it arrived over MCP.

## Conformance acceptance

Validate current and proposed envelope examples, missing/unknown fields, stale
revision, duplicate key/operation IDs, integer boundaries, unsupported operation,
lost-response replay, dry-run, paginated snapshot consistency and concurrent
writers. Then send the same operation through CLI and MCP and compare durable
state and structured error semantics. Compile/runtime responses must agree
with published schemas before treating this draft as a stable interface.
