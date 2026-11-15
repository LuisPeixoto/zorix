# Provider landscape

How the model APIs ZORIX targets in Phase 2 differ, what the adapters must handle, and why the adapters are written
over raw HTTP. Researched in November 2026 from the providers' official documentation; items marked *(verify)* came from
secondary sources or could not be confirmed from a primary page.

## The four API families

| | Anthropic Messages | OpenAI Responses | OpenAI-style Chat Completions | Gemini `generateContent` |
|---|---|---|---|---|
| Used for | Anthropic | OpenAI (first party) | OpenRouter, Groq, vLLM, LM Studio, **Ollama** `/v1` | Google Gemini |
| Auth | `x-api-key`, `anthropic-version: 2026-10-21` | `Authorization: Bearer` | `Authorization: Bearer` (none for most local servers) | `x-goog-api-key` |
| Request | `messages`, top-level `system`, `max_tokens` required | `input` items, `tools` | `messages`, `tools` | `contents`, `tools.functionDeclarations`, `systemInstruction` |
| Tool call in the answer | `tool_use` block (`id`, `name`, `input` object) | `function_call` item (`call_id`, `name`, `arguments` **JSON string**) | `tool_calls[]` (`id`, `function.arguments` **JSON string**) | `functionCall` part (`name`, `args` object; no id) |
| Tool result | `tool_result` block in **one** user message for all parallel calls | `function_call_output` item with the `call_id` | `role: tool` message with `tool_call_id` | `functionResponse` part |
| Streaming | SSE, typed events | SSE, typed events | SSE `data:` chunks, `[DONE]` | SSE or chunked JSON |
| Model-bound reasoning to replay | `thinking` blocks, bound to the conversation prefix | reasoning items returned with tool calls must be passed back | none | `thoughtSignature`, **required** on function-call turns (4xx otherwise) |
| Usage | `usage.input_tokens/output_tokens`, cache read/write | `usage` with reasoning tokens | `usage.prompt_tokens/completion_tokens` (stream: `stream_options.include_usage`) | `usageMetadata` |
| Refusal | `stop_reason: "refusal"` with `stop_details` | refusal output items | `finish_reason: "content_filter"` *(verify)* | `finishReason: SAFETY` |

## What each provider forces on an adapter

### Anthropic

- No official Rust SDK, so the adapter speaks HTTP. The reference for the current API says raw HTTP is the right route
  when no SDK exists.
- `anthropic-version: 2026-10-21`: a request that sends `temperature`, `top_p` or `top_k` returns **400 on every model**.
  The adapter never sends them. A `document` block directly in an assistant message and `text` blocks with `citations`
  in a user message are also rejected; the adapter never produces either.
- The default model of the family, `claude-opus-5-5`, **always thinks**: `thinking: {type: "disabled"}` and
  `budget_tokens` return 400. The adapter omits `thinking` and controls depth with `output_config.effort` (the default on
  that model is `medium`, so the adapter sets it explicitly). Forced `tool_choice` (`any`/`tool`) is a 400 as well.
- **Preserved thinking.** Thinking blocks record the conversation prefix that produced them. Editing earlier turns
  invalidates later blocks and, for newer accounts, the request fails. Appending is safe; so is removing all or a leading
  run of thinking blocks. Consequence: keep the provider's raw assistant turn and replay it unchanged to the same model.
- Safety classifiers can decline with HTTP 200 and `stop_reason: "refusal"`. The server-side `fallbacks` parameter
  (beta `server-side-fallback-2026-07-01`) re-runs the request on a fallback model; the adapter opts in and also maps a
  remaining refusal to a typed error so ZORIX's own router can fall back.
- Long outputs need streaming. Prompt caching: a top-level `cache_control` marks the stable prefix; `usage` reports cache
  reads and writes.
- Prices (cached 2026-11-03): Opus 5.5 $4 / $20 per million tokens, Sonnet 5.5 $2 / $10, Haiku 5.5 $0.10 / $0.50 (up to
  100K prompt tokens), 1M-token windows.

### OpenAI

- Current models **require the Responses API for tool calling**; Chat Completions stays supported mainly for older and
  compatible use *(verify the exact model list on OpenAI's page, which names it only for some models)*. Reasoning items
  returned next to tool calls must be passed back with the tool outputs.
- `strict: true` needs every property in `required` and `additionalProperties: false`; ZORIX's tool schemas are written
  that way, but the adapter does not force strictness on schemas it cannot prove are compatible.
- Arguments arrive as a JSON **string**; the adapter parses it and treats a parse failure as a model mistake (an error
  result the model sees), not as a crash.

### OpenAI-compatible servers and Ollama

- Same Chat Completions shape; differences are what they leave out. Ollama's `/v1/chat/completions` supports `tools` but
  not `tool_choice`; it exposes `/v1/models`. Native endpoints add what the compatibility layer lacks: `/api/tags` (what is
  installed) and `/api/show` (capabilities such as tools and vision, and the context length).
- Streaming with tools works on current Ollama versions; older client docs that say otherwise are stale.
- A model without the `tools` capability cannot be used by the loop: the catalogue marks it and the router skips it.
- No credentials for local servers; base URL on loopback.

### Gemini

- Gemini 3 and newer **require `thoughtSignature` to be returned** on function-call turns; a missing signature is a 4xx.
  With parallel calls only the first `functionCall` part carries one. The adapter replays the raw model turn.
- Function declarations use an OpenAPI-subset schema: no `additionalProperties`, a limited set of keywords. The adapter
  translates ZORIX's JSON Schemas and rejects what it cannot express instead of sending something different.
- Calls carry no ids; the adapter derives stable ids from position and name for the runtime.

## Rust crates considered

| Option | Verdict |
|---|---|
| `reqwest` 0.13 (rustls) + our own adapters | **Chosen.** Smallest surface; full control of raw turns, error classes and redaction |
| Rig (`rig-core`/`rig-agent`), `genai`, `async-openai` | Not used. They normalise turns in ways that lose what must be replayed (thinking blocks, thought signatures, reasoning items), hide the HTTP errors we route on, and add many dependencies. Rig remains a candidate for ideas, not for the core |
| A LiteLLM-style proxy | Not used. A server process the desktop app should not need |
| `keyring` 4.x for credentials | **Chosen** (see [ADR-0011](../adr/0011-keychain-credentials.md)) |
| `wiremock` for adapter tests | **Chosen** for offline, deterministic HTTP tests |

## Prices and capabilities

Providers do not publish prices in their APIs (only the Models API of one provider reports context windows and
capabilities). ZORIX ships a **curated catalogue** with an "as of" date and lets users override it. Where a price is
unknown the cost stays unknown, and a cost limit then warns instead of guessing.

## Sources

- Anthropic: the Claude API reference for the Messages API, model migration and API-version notes (version 2026-10-21),
  refusals and fallbacks, prompt caching.
- OpenAI: developers.openai.com — function calling guide; Responses API.
- Google: ai.google.dev/gemini-api/docs — thought signatures; function calling.
- Ollama: docs.ollama.com — API chat, OpenAI compatibility, tool calling, streaming.
- keyring: docs.rs/crate/keyring (4.x), lib.rs/crates/keyring.
