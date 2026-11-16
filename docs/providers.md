# Providers, models and routing

ZORIX talks to real AI models through **providers**. A run names a model; the **router** turns that name into a provider
call and, if you asked for it, tries another model when one fails. The runtime does not change: the same grants,
approvals, limits, checkpoints and events apply whichever model answers.

> **Data leaves your computer.** A cloud provider (Anthropic, OpenAI, Gemini, OpenRouter…) receives the conversation of
> the run: your prompt, the tool results and any file contents a tool returned. Ollama and other servers on `localhost`
> keep it on this machine. Use `--local-only` to forbid everything else.

## Providers

| Name | Kind | Talks to | Key |
|---|---|---|---|
| `anthropic` | `anthropic` | Messages API | yes |
| `openai` | `openai` | Responses API | yes |
| `gemini` | `gemini` | Generative Language API | yes |
| `ollama` | `ollama` | the local server, `http://127.0.0.1:11434` | no |

They exist without any configuration. Add more in `providers.toml` (see below): any server that speaks the Chat
Completions protocol (`kind = "openai-chat"`) works, such as OpenRouter, Groq, vLLM or LM Studio.

```
zorix provider list          # where each is, cloud or local, and whether it has a key
zorix provider test ollama   # asks the provider which models it offers; sends no conversation
```

## Credentials

Keys live in the **system keychain** (macOS Keychain, Windows Credential Manager, the Secret Service on Linux), under
the provider's name. They are never written to `providers.toml`, the database, events, logs or traces.

```
zorix provider set-key anthropic     # asks on the terminal with hidden input
echo "$KEY" | zorix provider set-key anthropic --stdin    # for scripts
zorix provider remove-key anthropic
```

The desktop app has the same in **Providers & keys**: the key goes from the field to the keychain and is never shown again.

An environment variable also works and wins over the keychain: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, or
the variable named by `api_key_env` for a custom provider. Set `ZORIX_SECRETS=env` to leave the keychain out (servers, CI).
`providers.toml` is refused if it contains anything that looks like a key.

## Naming a model

| You write | It means |
|---|---|
| `anthropic/claude-opus-5-5`, `ollama/qwen3:8b` | exactly that model. If it fails, the run fails: no silent switch |
| `route:<name>` | a route from `providers.toml`: an ordered list with fallback |
| `<alias>` | a name from `[alias]`, which points at any of these |
| `default` | the alias `default` if you set one; otherwise `anthropic/claude-opus-5-5` when Anthropic has a key; otherwise `demo` |
| `demo`, `script:<file>` | the simulated models of Phase 1 |

`zorix models` lists what is known; `zorix models --available` asks each provider that has a key (or runs here).
The model of every step, and every switch, is recorded in the events and shown in `zorix show` and the desktop run view.

## `providers.toml`

It lives in the data folder (`zorix data-dir`). Every part is optional.

```toml
local_only = false              # true: never use a provider that is not on this computer

[[provider]]                    # an extra server, or a change to a built-in one (same name)
name = "openrouter"
kind = "openai-chat"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[[model]]                       # tell ZORIX about a model it does not know (or correct its numbers)
provider = "openrouter"
id = "some/model"
context_window = 128000
max_output = 8192
tools = true

[alias]
default = "route:reliable"
fast = "anthropic/claude-haiku-5-5"

[[route]]
name = "reliable"
candidates = ["anthropic/claude-opus-5-5", "openrouter/some/model", "ollama/qwen3:8b"]
require = ["tools"]             # drop candidates that lack a capability: tools, vision
order = "listed"                # listed (default), best, cheapest, fastest

[[route]]
name = "tiered"                 # one model to start, a cheaper one to continue
candidates = ["anthropic/claude-sonnet-5-5"]
start = ["anthropic/claude-opus-5-5"]
continue = ["anthropic/claude-haiku-5-5"]
```

Base URLs must be `https://`, except `localhost` and other loopback addresses. Redirects are not followed.

## Fallback

The router filters the candidates (provider turned on, `local_only`, needed capabilities, the conversation fits the
window), orders them, and tries them in turn. It moves on after: unavailable or overloaded, rate limited, a bad or missing
key, model not found, context too long, or a refusal. It does **not** move on after an invalid request (it would fail
everywhere) or when you cancel. A model that just failed rests for a short time (longer after a rate limit that said how
long to wait) and goes last. Each switch is an `execution.model.fallback` event with the reason.

## Models and numbers

`models.json` ships with context windows, output limits, capabilities and, for Anthropic, prices, with an "as of" date.
Providers do not publish prices through their APIs, so an unknown price stays **unknown**: the run is not billed
by guess, and cost limits do not count what is not known. Discovery fills in the models a provider reports; for Ollama it
reads the real context length and capabilities, and ZORIX asks Ollama for at most 32 768 tokens of context.

## Replaying a model's own turn

Some providers bind a model's reasoning to the exact conversation that produced it (Anthropic's thinking blocks, Gemini's
thought signatures, OpenAI's encrypted reasoning items). ZORIX keeps each assistant turn as the provider sent it and sends
it back unchanged when the next call goes to the same model. If it has to drop old turns to fit a smaller window, or the
next step goes to another model, it sends the portable form instead. See [ADR-0012](adr/0012-native-turns-append-only.md).

## Limits of this version

- Answers arrive when complete; live text as it is generated is not shown yet (adapters stream internally).
- No classifier-based routing, embeddings, image or audio generation, or batch APIs.
- Prices are known only for some models.
- The Windows and Linux keychain paths are covered by CI builds, not by a hands-on run.
