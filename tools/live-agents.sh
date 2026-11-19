#!/usr/bin/env bash
# Checks of the agent engine through the real `zorix` command line in a throwaway data folder. Nothing here uses your accounts,
# your keychain or your data folder. Run it with `tools/live-tests.sh agents`.
#
#   scripted   always: a skill is loaded and its script runs in the sandbox, an agent cannot use a model outside its list,
#              a template installs and chats, an evaluation runs, and the agent file round-trips
#   Ollama     when it runs here: a template agent answers with a real local model, and its evaluation suite is tried
#              (set ZORIX_LIVE_OLLAMA_MODEL to a model that supports tools; default qwen3:1.7b). Quality is reported, not asserted.
#
# Cloud providers are not used here. Try an agent on one yourself: `zorix run --agent writer --model anthropic/claude-opus-5-5 "…"`.
set -uo pipefail
cd "$(dirname "$0")/.."

cargo build -q -p zorix-cli || { echo "could not build zorix" >&2; exit 1; }
Z="$PWD/target/debug/zorix"
WORK="$(mktemp -d)"
export ZORIX_HOME="$WORK/home" ZORIX_SECRETS=env ZORIX_TEST_SECRETS_FILE="$WORK/secrets.json"
trap 'rm -rf "${WORK:?}"' EXIT

pass=0; fail=0; skipped=0
ok()   { echo "  ok    $1"; pass=$((pass + 1)); }
bad()  { echo "  FAIL  $1"; fail=$((fail + 1)); [ -n "${2:-}" ] && echo "$2" | sed 's/^/        /' | head -30; }
skip() { echo "  skip  $1"; skipped=$((skipped + 1)); }
run()  { OUT="$("$@" 2>&1)"; RC=$?; }

last_results() {
  python3 - "$ZORIX_HOME/zorix.sqlite" <<'PY'
import json, sqlite3, sys
for row in sqlite3.connect(sys.argv[1]).execute("select doc from tool_calls order by rowid"):
    print(json.dumps(json.loads(row[0]).get("result")))
PY
}

echo "== a skill and its script, in the sandbox"
mkdir -p "$WORK/skills/add-numbers/scripts"
cat > "$WORK/skills/add-numbers/SKILL.md" <<'MD'
---
name: add-numbers
description: "Adds two numbers with a script. Use when asked to add."
---

# Add numbers

Run `scripts/add.js` with the two numbers as arguments.
MD
cat > "$WORK/skills/add-numbers/scripts/add.js" <<'JS'
console.log(JSON.stringify({ sum: 2 + 3 }));
JS
run "$Z" skill import "$WORK/skills/add-numbers"
[ $RC -eq 0 ] && ok "the skill folder was imported" || bad "skill import" "$OUT"
run "$Z" agent create adder --model script --access code:javascript --skill add-numbers --instructions "Use the skill to add."
[ $RC -eq 0 ] && ok "an agent was made with the skill pinned" || bad "agent create" "$OUT"
printf '%s' '[{"call":{"calls":[{"tool":"skill.load","arguments":{"name":"add-numbers"}}]}},{"call":{"calls":[{"tool":"skill.run","arguments":{"name":"add-numbers","script":"scripts/add.js","args":["2","3"]}}]}},{"say":"Done."}]' > "$WORK/adder.json"
run "$Z" run --agent adder --model "script:$WORK/adder.json" "add 2 and 3" --approve yes
if [ $RC -eq 0 ] && last_results | grep -q 'sum'; then ok "the agent loaded the skill and ran its script (sum came back)"; else bad "skill run" "$OUT"; fi
id="$("$Z" --json list --agent adder 2>/dev/null | python3 -c 'import json,sys; print(json.loads(sys.stdin.read().splitlines()[0])[0]["id"])')"
run "$Z" show "$id"
echo "$OUT" | grep -q "Agent      adder v1" && ok "the run records the agent and its version" || bad "show" "$OUT"

echo "== an agent cannot use a model outside its list"
cat > "$WORK/picky.md" <<'MD'
---
zorix: agent/1
slug: picky
name: "Picky"
models:
  default: "demo"
  allowed:
    - "anthropic/*"
---

Answer briefly.
MD
run "$Z" agent import "$WORK/picky.md"
run "$Z" run --agent picky "hi"
if [ $RC -ne 0 ] && echo "$OUT" | grep -qi "may not use"; then ok "the demo model was refused with a reason"; else bad "allow-list" "$OUT"; fi

echo "== a template, its chat and its evaluation"
run "$Z" agent add --template data-analyst
[ $RC -eq 0 ] && ok "data-analyst was added with its skill and cases" || bad "add template" "$OUT"
run "$Z" run --agent data-analyst --model demo "What is 17*23?"
[ $RC -eq 0 ] && ok "the template answered through the demo model" || bad "template run" "$OUT"
run "$Z" agent export data-analyst -o "$WORK/da.md" --with-skills "$WORK/da-skills"
[ -f "$WORK/da.md" ] && [ -f "$WORK/da-skills/csv-profiling/scripts/profile.py" ] && ok "the agent and its skill were exported" || bad "export" "$OUT"
OTHER="$WORK/other"
OUT="$(ZORIX_HOME="$OTHER" "$Z" agent import "$WORK/da.md" --skills "$WORK/da-skills" 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "the file imports elsewhere with its skill" || bad "import" "$OUT"
printf '%s' '[{"name":"multiplies","prompt":"What is 17*23?","tags":["smoke"],"checks":[{"kind":"contains","text":"391"}]}]' > "$WORK/cases.json"
run "$Z" agent create calc --model demo --tool math.evaluate --instructions "Calculate."
run "$Z" eval add calc "$WORK/cases.json"
run "$Z" eval run calc
if [ $RC -eq 0 ] && echo "$OUT" | grep -q "PASS  multiplies"; then ok "an evaluation ran and passed"; else bad "eval run" "$OUT"; fi

echo "== a real local model (Ollama)"
HOST="${OLLAMA_HOST:-http://localhost:11434}"
MODEL="${ZORIX_LIVE_OLLAMA_MODEL:-qwen3:1.7b}"
if curl -fsS -m 3 "$HOST/api/tags" 2>/dev/null | grep -q "\"$MODEL\""; then
  run "$Z" agent add --template writer
  run "$Z" run --agent writer --model "ollama/$MODEL" "Rewrite in plain words: It should be noted that the office will be closed on Friday." --max-iterations 4
  if [ $RC -eq 0 ] && [ -n "$(echo "$OUT" | tail -1)" ]; then ok "writer answered with $MODEL: $(echo "$OUT" | tail -3 | head -1 | cut -c1-90)"; else bad "ollama run" "$OUT"; fi
  run "$Z" eval run writer --model "ollama/$MODEL" --tag smoke --offline
  echo "$OUT" | sed 's/^/        /' | head -12
  echo "  info  the suite's result depends on the model; it is shown, not asserted"
else
  skip "Ollama is not running here, or $MODEL is not pulled (ollama serve; ollama pull $MODEL)"
fi

echo
echo "passed: $pass, failed: $fail, skipped: $skipped"
[ "$fail" -eq 0 ]
