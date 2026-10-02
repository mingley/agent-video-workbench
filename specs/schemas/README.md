# Draft wire schemas

These JSON Schema 2020-12 documents are design artifacts for the
[agent protocol](../../docs/specs/agent-protocol.md). They preserve the prototype's
AgentCut batch naming and `ok` / `apiVersion` / `result` / `error` response shape.
They are not generated from Rust types and are not yet the runtime schema API.

| File | Validates | Does not validate |
| --- | --- | --- |
| [edit-request.schema.json](edit-request.schema.json) | Batch envelope, required fields, operation envelope, basic types/limits | Operation existence, each `params` schema, unique IDs, exact time arithmetic, source availability, protected coverage or current revision |
| [response.schema.json](response.schema.json) | Exclusive success/error envelope, error structure and optional metadata | Command-specific `result` semantics, actual job completion, media quality or durable commit |

The stricter request limits and optional response fields are proposed contract
rules. Current Rust parsing may accept a broader request or return a smaller
response. Operation/property schemas must be generated or checked against their
Rust registry before release; successful envelope validation alone cannot make
an edit executable. JSON parsing must separately reject duplicate object keys.
Enable format validation when checking UUIDs.

Examples use synthetic identifiers and an illustrative project at revision 12
containing `caption_demo`. The draft size change uses existing core operation
names; substitute real IDs and validate against a real project before applying.
Job/resume examples describe planned APIs. The `aaaaaaaa...` hash is a placeholder.

| Example | Schema |
| --- | --- |
| [Edit request](../../examples/protocol/edit-request.json) | Edit request |
| [Edit success](../../examples/protocol/edit-success.json) | Response; adds proposed richer outcome fields |
| [Revision conflict](../../examples/protocol/edit-error.json) | Response; proposed recovery details |
| [Job still verifying](../../examples/protocol/job-status.json) | Response; complete encoding does not imply delivery success |
| [Resume context](../../examples/protocol/resume-status.json) | Response; older artifacts remain explicitly associated with older revisions |

Before adopting the draft, run structural validation on these examples and
negative cases (missing key, invalid revision/UUID, mixed success/error), then
conformance tests against actual CLI/MCP outputs and domain operations. The
documentation change validates structure only; no example demonstrates an
implemented media job or hosted-agent integration.
