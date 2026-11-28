#!/usr/bin/env bash
# Run the tests that talk to real providers. They are ignored by default and never run in CI.
#
#   tools/live-tests.sh            Ollama (if it runs) and every cloud provider that has a key
#   tools/live-tests.sh ollama     only Ollama
#   tools/live-tests.sh anthropic  only that provider (also: openai, gemini)
#   tools/live-tests.sh integrations MCP servers, GitHub (public) and the sandbox, through the real command line
#   tools/live-tests.sh agents     the agent engine through the real command line (skills, allow-list, templates, evaluations)
#                                  and, when Ollama runs, a real local model
#   tools/live-tests.sh orchestration teams and delegation through the real command line (parallel, nested, approvals up the tree,
#                                  cancel, budgets) and, only when Ollama already runs, a real local model
#   tools/live-tests.sh workflows  workflows through the real command line (publish, approvals, versions, `zorix serve` and a signed
#                                  webhook); the demo model only
#   tools/live-tests.sh projects   projects and tasks through the real command line (a task runs when it moves to In progress, a
#                                  dependency waits, `zorix serve` starts it, the manager is asked); the demo model only
#   tools/live-tests.sh workforce  the workforce through the real command line (workers in departments, levels and budgets,
#                                  the manager's plan and report, the schedule); the demo model only
#   tools/live-tests.sh phase11    reading scans (downloads the real OCR models), a watched folder, attached files and OpenTelemetry export
#                                  through the real command line on a throwaway data folder; `phase11 ocr|watch|attach|telemetry` runs one
#   tools/live-tests.sh embeddings embeddings and memory: Ollama (all-minilm, plus qwen3:1.7b for the run that cites a
#                                  document) and, with a stored key, OpenAI and Gemini embeddings
#
# Embeddings: `ollama pull all-minilm`.
# Ollama: start it (`ollama serve`) and `ollama pull qwen3:1.7b`, or set ZORIX_LIVE_OLLAMA_MODEL to a model that
# supports tools. Cloud: store a key with `zorix provider set-key <name>` (or set ANTHROPIC_API_KEY, OPENAI_API_KEY,
# GEMINI_API_KEY); a provider without a key is skipped. Cloud tests cost a few cents. Keys are never printed.
set -euo pipefail
cd "$(dirname "$0")/.."

filter="${1:-}"
args=(--test live -- --ignored --nocapture --test-threads=1)
[ -n "$filter" ] && args+=("$filter")

if [ "$filter" = integrations ]; then
  exec tools/live-integrations.sh
fi

if [ "$filter" = agents ]; then
  exec tools/live-agents.sh
fi

if [ "$filter" = orchestration ]; then
  exec tools/live-orchestration.sh
fi

if [ "$filter" = workflows ]; then
  exec tools/live-workflows.sh
fi

if [ "$filter" = projects ]; then
  exec tools/live-projects.sh
fi

if [ "$filter" = workforce ]; then
  exec tools/live-workforce.sh
fi

if [ "$filter" = phase11 ]; then
  exec tools/live-memory.sh "${2:-all}"
fi

if [ "$filter" = embeddings ]; then
  if ! curl -fsS -m 3 "${OLLAMA_HOST:-http://localhost:11434}/api/tags" >/dev/null 2>&1; then
    echo "Ollama is not running here. Start it with 'ollama serve' (and 'ollama pull all-minilm')." >&2
    exit 1
  fi
  cargo test -p zorix-memory --test live -- --ignored --nocapture --test-threads=1
  exec cargo test -p zorix-providers --test live -- --ignored --nocapture --test-threads=1 embeddings
fi

if [ -z "$filter" ] || [ "$filter" = ollama ]; then
  if ! curl -fsS -m 3 "${OLLAMA_HOST:-http://localhost:11434}/api/tags" >/dev/null 2>&1; then
    echo "Ollama is not running here. Start it with 'ollama serve' to include it." >&2
    [ "$filter" = ollama ] && exit 1
  fi
fi

exec cargo test -p zorix-providers "${args[@]}"
