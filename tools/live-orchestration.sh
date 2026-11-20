#!/usr/bin/env bash
# Checks of multi-agent orchestration through the real `zorix` command line in a throwaway data folder. Nothing here uses your
# accounts, your keychain or your data folder. Run it with `tools/live-tests.sh orchestration`.
#
#   scripted   always: a team installs, a lead asks two members at once, delegation nests, an approval asked by a sub-run is
#              answered from the run you started, cancelling the root stops the whole tree, and a budget stops the parent
#   Ollama     only when it already runs here (this script never starts it): the research desk answers with a real local model.
#              Quality is reported, not asserted. Set ZORIX_LIVE_OLLAMA_MODEL (default qwen3:1.7b; it must support tools).
#
# The scripted model is shared by the whole run, lead and sub-runs alike, so each script lists the turns in the order they are used.
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
# The newest root run, and a field of a JSON list.
root_id() { "$Z" --json list 2>/dev/null | python3 -c 'import json,sys; print(json.loads(sys.stdin.read().splitlines()[-1])[0]["id"])'; }
count()   { "$Z" --json "$@" 2>/dev/null | python3 -c 'import json,sys; print(len(json.loads(sys.stdin.read().splitlines()[-1])))'; }
script()  { printf '%s' "$2" > "$WORK/$1.json"; echo "script:$WORK/$1.json"; }
fresh()   { rm -rf "${ZORIX_HOME:?}"; }

DELEGATE_TWO='{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"researcher","task":"Find sources."},{"agent":"writer","task":"Draft an opening."}]}}]}}'

echo "== a ready-made team and a lead that asks two members at once"
run "$Z" team add --template research-desk
if [ $RC -eq 0 ] && echo "$OUT" | grep -q "Added the team research-desk"; then ok "the research desk was added with its three agents"; else bad "team add" "$OUT"; fi
# The starter members use the app's default model; a throwaway folder has none, so point them at the demo model.
for m in researcher writer; do
  "$Z" agent export "$m" -o "$WORK/$m.md" >/dev/null 2>&1
  python3 - "$WORK/$m.md" <<'PY'
import re, sys
s = open(sys.argv[1]).read()
open(sys.argv[1], "w").write(re.sub(r'(models:\n  default: )"[^"]*"', r'\1"demo"', s, count=1))
PY
  "$Z" agent import "$WORK/$m.md" --note "demo model for this check" >/dev/null 2>&1
done
M=$(script parallel "[$DELEGATE_TWO,{\"say\":\"Researcher part done.\"},{\"say\":\"Writer part done.\"},{\"say\":\"Final: both parts are in.\"}]")
run "$Z" run --team research-desk --model "$M" "Prepare a brief."
if [ $RC -eq 0 ] && echo "$OUT" | grep -q "Final: both parts are in."; then ok "the lead waited for both and answered"; else bad "team run" "$OUT"; fi
ROOT=$(root_id)
run "$Z" tree "$ROOT"
if [ "$(echo "$OUT" | grep -c 'v[0-9]')" -eq 3 ] && echo "$OUT" | grep -q "researcher" && echo "$OUT" | grep -q "writer"; then ok "the tree shows the lead and both members"; else bad "tree" "$OUT"; fi
[ "$(count list)" -eq 1 ] && [ "$(count list --all)" -eq 3 ] && ok "list shows the root only, and --all shows the sub-runs" || bad "list" "$(count list) / $(count list --all)"
run "$Z" show "$ROOT"
echo "$OUT" | grep -q "Sub-runs   2" && ok "show counts the sub-runs" || bad "show" "$OUT"
"$Z" --json list --all 2>/dev/null | python3 -c '
import json, sys
runs = json.loads(sys.stdin.read().splitlines()[-1])
cost = lambda r: r["usage"]["cost_micros"]
root = [r for r in runs if not r.get("parent")][0]
kids = sum(cost(r) for r in runs if r.get("parent"))
sys.exit(0 if kids > 0 and cost(root) >= kids else 1)' && ok "the root's cost includes both members" || bad "usage roll-up"

echo "== delegation nests, one level at a time"
fresh
# A sub-run can never do more than the run that asked it, so the top must be allowed to reach what its helpers reach.
"$Z" agent create other --model script --instructions "You are other." >/dev/null 2>&1
for a in leaf mid top; do
  access=()
  [ "$a" = mid ] && access=(--access agent:leaf)
  [ "$a" = top ] && access=(--access agent:mid --access agent:leaf)
  "$Z" agent create "$a" --model script --instructions "You are $a." ${access[@]+"${access[@]}"} >/dev/null 2>&1
done
M=$(script nested '[{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"mid","task":"Handle this."}]}}]}},{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"leaf","task":"Do the detail."}]}}]}},{"say":"Detail done."},{"say":"Middle done."},{"say":"Final: all levels done."}]')
run "$Z" run --agent top --model "$M" "Start."
if [ $RC -eq 0 ] && echo "$OUT" | grep -q "Final: all levels done."; then ok "a run asked a run that asked a run"; else bad "nested run" "$OUT"; fi
run "$Z" tree "$(root_id)"
if [ "$(echo "$OUT" | grep -c 'v[0-9]')" -eq 3 ] && echo "$OUT" | grep -q "│\|   └─\|    └─"; then ok "the tree is three levels deep"; else bad "nested tree" "$OUT"; fi
# A member the lead may not ask is refused with a reason, and the model is told.
M=$(script refused '[{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"other","task":"Not on my list."}]}}]}},{"say":"Final: it was refused."}]')
run "$Z" run --agent top --model "$M" "Try."
if echo "$OUT" | grep -qi "refused\|not allowed\|may not"; then ok "asking an agent outside the lead's access was refused with a reason"; else bad "refusal" "$OUT"; fi

echo "== an approval asked below is answered from the run you started"
fresh
"$Z" agent create boss --model script --tool notes.put --access agent:helper --instructions "You lead." >/dev/null 2>&1
"$Z" agent create helper --model script --tool notes.put --instructions "You help." >/dev/null 2>&1
"$Z" team create duo --lead boss --member helper >/dev/null 2>&1
M=$(script ask '[{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"helper","task":"Save a note."}]}}]}},{"call":{"calls":[{"tool":"notes.put","arguments":{"key":"k","value":"v"}}]}}]')
run "$Z" run --team duo --model "$M" --approve never "Save a note."
if [ $RC -eq 3 ] && echo "$OUT" | grep -q "zorix approve"; then ok "the run stopped and said how to answer"; else bad "waiting" "$OUT"; fi
ROOT=$(root_id)
M=$(script answer '[{"say":"Saved it."},{"say":"Final: saved."}]')
run "$Z" approve "$ROOT" --model "$M"
if [ $RC -eq 0 ] && echo "$OUT" | grep -q "Final: saved."; then ok "approving the root resumed the sub-run and then the root"; else bad "approve" "$OUT"; fi

echo "== cancelling the root stops the whole tree"
fresh
"$Z" agent create boss --model script --tool notes.put --access agent:helper --instructions "You lead." >/dev/null 2>&1
"$Z" agent create helper --model script --tool notes.put --instructions "You help." >/dev/null 2>&1
"$Z" team create duo --lead boss --member helper >/dev/null 2>&1
M=$(script ask2 '[{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"helper","task":"Save a note."}]}}]}},{"call":{"calls":[{"tool":"notes.put","arguments":{"key":"k","value":"v"}}]}}]')
run "$Z" run --team duo --model "$M" --approve never "Save a note."
ROOT=$(root_id)
run "$Z" cancel "$ROOT"
# Exit code 4 means "cancelled", which is what was asked for.
[ $RC -eq 4 ] && ok "the root was cancelled (exit 4)" || bad "cancel (exit $RC)" "$OUT"
run "$Z" tree "$ROOT"
if [ "$(echo "$OUT" | grep -c cancelled)" -ge 2 ]; then ok "the sub-run was cancelled with it"; else bad "cascade" "$OUT"; fi

echo "== a budget stops the parent"
fresh
"$Z" agent create boss --model script --tool notes.put --access agent:helper --instructions "You lead." >/dev/null 2>&1
"$Z" agent create helper --model script --tool notes.put --instructions "You help." >/dev/null 2>&1
"$Z" team create duo --lead boss --member helper >/dev/null 2>&1
M=$(script budget '[{"call":{"calls":[{"tool":"agents.delegate","arguments":{"tasks":[{"agent":"helper","task":"Save a note."}]}}]}},{"call":{"calls":[{"tool":"notes.put","arguments":{"key":"k","value":"v"}}]}},{"say":"Saved."},{"say":"Final: should not get here."}]')
run "$Z" run --team duo --model "$M" --approve yes --max-tool-calls 1 "Save a note."
if [ $RC -ne 0 ] && echo "$OUT" | grep -qi "tool call"; then ok "the sub-run's tool call counted against the parent's limit"; else bad "budget" "$OUT"; fi

echo "== the research desk with a real local model (Ollama, only if it already runs)"
HOST="${OLLAMA_HOST:-http://localhost:11434}"
MODEL="${ZORIX_LIVE_OLLAMA_MODEL:-qwen3:1.7b}"
if curl -fsS -m 3 "$HOST/api/tags" 2>/dev/null | grep -q "\"$MODEL\""; then
  fresh
  "$Z" team add --template research-desk >/dev/null 2>&1
  run "$Z" run --team research-desk --model "ollama/$MODEL" --max-iterations 6 "In two sentences: why is the sky blue? Ask the writer to polish the wording."
  echo "$OUT" | tail -4 | sed 's/^/        /'
  run "$Z" tree "$(root_id)"
  echo "$OUT" | sed 's/^/        /' | head -6
  echo "  info  what a small model does with delegation depends on the model; it is shown, not asserted"
else
  skip "Ollama is not running here, or $MODEL is not pulled (this script never starts it)"
fi

echo
echo "passed: $pass, failed: $fail, skipped: $skipped"
[ "$fail" -eq 0 ]
