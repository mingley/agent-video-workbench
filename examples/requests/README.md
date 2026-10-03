# Current service requests

These are executable **0.3.1** service envelopes for
[the first-edit walkthrough](../../docs/first-edit.md). They target a fresh
`project` containing assets `source` and `font`, at revision 2. Use a recording
at least ten seconds long. The transcript text is illustrative; review and
replace it for your recording before import.

| Order | Request | Required revision | Resulting revision |
| --- | --- | --- | --- |
| 1 | [transcript-import.json](transcript-import.json) | 2 | 3 |
| 2 | [compose.json](compose.json) | 3 | 4 |
| 3 | [variant.json](variant.json) | 4 | 5 |

Pass complete files to `avw --workspace ROOT request FILE`, or their JSON as
arguments to the MCP `avw` tool. The flat `compose`/`studio`/`transcript-import`
aliases instead expect their inner edit/transcript object in `--request`.

Reusing an unchanged request/key replays its outcome. For a different project
or edit, read the current revision, substitute actual IDs and use a new key.
[The generated schema](../../specs/schemas/service-request.schema.json) defines
the contract; [the interface reference](../../docs/agent-api.md) explains replies.
