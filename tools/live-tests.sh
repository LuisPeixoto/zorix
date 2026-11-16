#!/usr/bin/env bash
# Run the tests that talk to real providers. They are ignored by default and never run in CI.
#
#   tools/live-tests.sh            Ollama (if it runs) and every cloud provider that has a key
#   tools/live-tests.sh ollama     only Ollama
#   tools/live-tests.sh anthropic  only that provider (also: openai, gemini)
#
# Ollama: start it (`ollama serve`) and `ollama pull qwen3:1.7b`, or set ZORIX_LIVE_OLLAMA_MODEL to a model that
# supports tools. Cloud: store a key with `zorix provider set-key <name>` (or set ANTHROPIC_API_KEY, OPENAI_API_KEY,
# GEMINI_API_KEY); a provider without a key is skipped. Cloud tests cost a few cents. Keys are never printed.
set -euo pipefail
cd "$(dirname "$0")/.."

filter="${1:-}"
args=(--test live -- --ignored --nocapture --test-threads=1)
[ -n "$filter" ] && args+=("$filter")

if [ -z "$filter" ] || [ "$filter" = ollama ]; then
  if ! curl -fsS -m 3 "${OLLAMA_HOST:-http://localhost:11434}/api/tags" >/dev/null 2>&1; then
    echo "Ollama is not running here. Start it with 'ollama serve' to include it." >&2
    [ "$filter" = ollama ] && exit 1
  fi
fi

exec cargo test -p zorix-providers "${args[@]}"
