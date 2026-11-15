# ADR-0008: An MCP-shaped tool contract

Status: accepted (Phase 1)

## Context

Phase 4 brings MCP and external integrations; built-in and external tools should look the same to the runtime.

## Decision

`ToolSpec` mirrors MCP's tool definition (`name`, `description`, `input_schema`, `output_schema`, annotations) and adds
ZORIX fields: stable `id`, `required_permissions`, `timeout`, `max_output_bytes`. Arguments are validated against
`input_schema` before execution and the result against `output_schema` (when present). `Tool::call(ctx, args)` returns
JSON or a `ToolError` that is either *retryable* or *final*. Tools get a `ToolContext` (execution id, grants view,
cancellation token, workspace root) and no ambient access to the environment.

## Consequences

- An MCP adapter implements `Tool` by forwarding `tools/call`; no engine change.
- Output is truncated at `max_output_bytes` *before* serialisation, with a marker, never cut mid-JSON.
