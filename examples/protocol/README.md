# Historical protocol design examples

The JSON files here illustrate the October 2, 2026
[protocol draft](../../docs/specs/agent-protocol.md). They use synthetic IDs,
placeholder hashes and proposed response metadata. They are **not executable
0.3.1 requests or snapshots of current CLI/MCP replies**.

For example, `requestId`, `warnings`, detailed progress and the resume fields
shown here are not guaranteed runtime fields. Current jobs are polled by their
durable `id`; clients retrieve `artifact` for verified bytes and hashes.

Use [current request examples](../requests/README.md),
[the interface reference](../../docs/agent-api.md) and
[the generated schema](../../specs/schemas/service-request.schema.json) when
building an agent integration. The draft schemas are described separately in
[the schema index](../../specs/schemas/README.md).
