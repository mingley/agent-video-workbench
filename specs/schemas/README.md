# Runtime and design schemas

## Current 0.3.1 contract

[service-request.schema.json](service-request.schema.json) is generated from the
Rust `Request` type. `avw schema` returns it in `result`; MCP `tools/list` uses
it for the `avw` tool. `scripts/check-schema.py` compares the checked-in schema
with an executable. CLI aliases map to the same service.

The schema covers complete service envelopes and typed creator workflows.
Raw `apply` embeds a pinned AgentCut operation batch; `describe OPERATION`
provides property discovery and examples, and dry-run checks actual domain
semantics, exact time, source policy and revision. The parser additionally
rejects duplicate keys and integers outside the exact JSON range.

Start with [current executable requests](../../examples/requests/README.md),
[the first-edit walkthrough](../../docs/first-edit.md) and
[the interface reference](../../docs/agent-api.md). JSON Schema validation alone
cannot establish source availability, job completion or media quality.

## Historical design artifacts

These JSON Schema 2020-12 files support the
[protocol design](../../docs/specs/agent-protocol.md). They are not generated
runtime contracts, and their optional metadata does not guarantee fields in a
current response. Enable format validation when checking UUIDs.

| File | Draft scope | Requires application validation |
| --- | --- | --- |
| [edit-request.schema.json](edit-request.schema.json) | AgentCut batch envelope, operation envelope and basic limits | Operation/property semantics, unique IDs, time, source availability, protection and revision |
| [response.schema.json](response.schema.json) | Success/error envelope and proposed optional metadata | Command-specific result fields, durable commit, job completion and verified media |

[Historical examples](../../examples/protocol/README.md) use a synthetic
revision-12 project and placeholder hashes. Their richer success, conflict,
job-progress and resume shapes are design illustrations. They are not current
service envelopes or runtime transcripts. Preserve them for design context;
use generated requests and observed service results for integrations.
