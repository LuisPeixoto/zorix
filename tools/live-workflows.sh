#!/usr/bin/env bash
# Checks of workflows through the real `zorix` command line in a throwaway data folder. Nothing here uses your accounts, your
# keychain or your data folder, and every model is the built-in demo. Run it with `tools/live-tests.sh workflows`.
#
#   scripted   always: a workflow imports, publishes and runs; an agent step feeds an approval that stops the run; the answer must
#              be given and resumes it; a refusal fails the run; a test run asks before an action; a message that runs without
#              asking needs a confirmation when publishing; versions diff and roll back; and `zorix serve` receives a webhook
#              (wrong secret 404, wrong signature 401, right one starts a run, a body over 1 MB is refused 413).
set -uo pipefail
cd "$(dirname "$0")/.."

cargo build -q -p zorix-cli || { echo "could not build zorix" >&2; exit 1; }
Z="$PWD/target/debug/zorix"
WORK="$(mktemp -d)"
export ZORIX_HOME="$WORK/home" ZORIX_SECRETS=env ZORIX_TEST_SECRETS_FILE="$WORK/secrets.json"
SERVE_PID=""
trap '[ -n "$SERVE_PID" ] && kill "$SERVE_PID" 2>/dev/null; rm -rf "${WORK:?}"' EXIT

pass=0; fail=0
ok()  { echo "  ok    $1"; pass=$((pass + 1)); }
bad() { echo "  FAIL  $1"; fail=$((fail + 1)); [ -n "${2:-}" ] && echo "$2" | sed 's/^/        /' | head -30; }
run() { OUT="$("$@" 2>&1)"; RC=$?; }
has() { echo "$OUT" | grep -q -- "$1"; }
wait_id() { "$Z" --json workflow waits 2>/dev/null | python3 -c 'import json,sys; w=json.loads(sys.stdin.read().splitlines()[-1]); print(w[0]["id"] if w else "")'; }

"$Z" agent add --template researcher >/dev/null 2>&1

cat > "$WORK/flow.workflow.json" <<'JSON'
{
  "zorix": "workflow/1",
  "slug": "flow",
  "workflow": {
    "name": "Flow",
    "steps": [
      {"id": "start", "title": "Start by hand", "kind": {"type": "manual"}},
      {"id": "hello", "title": "Say hello", "kind": {"type": "agent", "agent": "researcher", "model": "demo", "prompt": "Say hello to {{trigger.payload.who}}."}},
      {"id": "ok", "title": "You approve", "kind": {"type": "approval", "message": "Keep {{steps.hello.output.text}}?", "show": ["steps.hello.output.text"]}}
    ],
    "edges": [{"from": "start", "to": "hello"}, {"from": "hello", "to": "ok"}]
  }
}
JSON

echo "== import, publish and run a workflow that stops for a person"
run "$Z" workflow import "$WORK/flow.workflow.json"
if [ $RC -eq 0 ] && has "Made flow"; then ok "the file imported as a draft"; else bad "import" "$OUT"; fi
run "$Z" workflow run flow --input '{"who":"Luis"}' --model demo
if [ $RC -ne 0 ] && has "not been published\|no published version"; then ok "a draft is not run by a trigger"; else bad "unpublished run" "$OUT"; fi
run "$Z" workflow publish flow --note first
if [ $RC -eq 0 ] && has "Published flow as v1"; then ok "published as v1"; else bad "publish" "$OUT"; fi
run "$Z" workflow run flow --input '{"who":"Luis"}' --model demo
if [ $RC -eq 3 ] && has "waiting for approval" && has "Hello\|hello"; then ok "the agent step ran and the approval stopped the run (exit 3)"; else bad "run" "$OUT"; fi
W=$(wait_id)
run "$Z" workflow answer "$W" --approve --model demo
if [ $RC -eq 0 ] && has "approved by"; then ok "the answer resumed the run and it finished"; else bad "answer" "$OUT"; fi
run "$Z" workflow answer "$W" --approve --model demo
if [ $RC -ne 0 ] && has "already answered"; then ok "an answered wait cannot be answered again"; else bad "second answer" "$OUT"; fi

echo "== a refusal fails the run, a test run asks first"
"$Z" workflow run flow --input '{"who":"Ana"}' --model demo >/dev/null 2>&1
W=$(wait_id)
run "$Z" workflow answer "$W" --refuse --note "not now" --model demo
if [ $RC -eq 1 ] && has "Refused by"; then ok "refusing failed the run (exit 1) and said who"; else bad "refuse" "$OUT"; fi

echo "== a message that runs without asking needs a confirmation when publishing"
cat > "$WORK/notify.workflow.json" <<'JSON'
{
  "zorix": "workflow/1",
  "slug": "notify",
  "workflow": {
    "name": "Notify",
    "steps": [
      {"id": "start", "title": "Start by hand", "kind": {"type": "manual"}},
      {"id": "post", "title": "Post to #ops", "kind": {"type": "send", "service": "slack", "channel": "#ops", "text": "hello", "ask": "never"}}
    ],
    "edges": [{"from": "start", "to": "post"}]
  }
}
JSON
"$Z" workflow import "$WORK/notify.workflow.json" >/dev/null 2>&1
run "$Z" workflow publish notify
if [ $RC -ne 0 ] && has "confirm-unattended"; then ok "publishing without a confirmation was refused"; else bad "publish unattended" "$OUT"; fi
run "$Z" workflow publish notify --confirm-unattended
if [ $RC -eq 0 ] && has "Published notify as v1"; then ok "publishing with a confirmation worked"; else bad "publish confirmed" "$OUT"; fi
run "$Z" workflow run notify --test --model demo
if [ $RC -eq 3 ] && has "waiting to be allowed"; then ok "a test run asked before sending"; else bad "test run" "$OUT"; fi

echo "== versions"
"$Z" workflow export flow -o "$WORK/flow2.workflow.json" >/dev/null 2>&1
sed -i.bak 's/Say hello to/Greet/' "$WORK/flow2.workflow.json" && rm -f "$WORK/flow2.workflow.json.bak"
"$Z" workflow import "$WORK/flow2.workflow.json" >/dev/null 2>&1
run "$Z" workflow diff flow 1 draft
if has "^+ .*Greet" && has "^- .*Say hello to"; then ok "the diff shows the changed prompt"; else bad "diff" "$OUT"; fi
"$Z" workflow publish flow >/dev/null 2>&1
run "$Z" workflow rollback flow 1
if [ $RC -eq 0 ] && has "Published v3"; then ok "rolling back published v1 again as v3"; else bad "rollback" "$OUT"; fi

echo "== serve: schedules, events and a webhook"
cat > "$WORK/hook.workflow.json" <<'JSON'
{
  "zorix": "workflow/1",
  "slug": "hook",
  "workflow": {
    "name": "Hook",
    "steps": [
      {"id": "in", "title": "A request arrives", "kind": {"type": "webhook", "source": "GitHub", "label": "issue labelled", "filter": "action == \"labeled\"", "signed": true}},
      {"id": "hello", "title": "Say hello", "kind": {"type": "agent", "agent": "researcher", "model": "demo", "prompt": "Note: {{trigger.payload.issue.title}}"}}
    ],
    "edges": [{"from": "in", "to": "hello"}]
  }
}
JSON
"$Z" workflow import "$WORK/hook.workflow.json" >/dev/null 2>&1
"$Z" workflow publish hook >/dev/null 2>&1
"$Z" workflow enable hook >/dev/null 2>&1
HOOK=$("$Z" --json workflow webhook hook --bind 127.0.0.1:17788 2>/dev/null | python3 -c 'import json,sys; d=json.loads(sys.stdin.read().splitlines()[-1]); print(d["url"], d["signing_secret"])')
URL="${HOOK% *}"; SECRET="${HOOK#* }"
"$Z" serve --model demo --bind 127.0.0.1:17788 >"$WORK/serve.log" 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 40); do curl -s -o /dev/null --noproxy '*' "http://127.0.0.1:17788/" 2>/dev/null && break; sleep 0.25; done
BODY='{"action":"labeled","issue":{"title":"Checkout fails"}}'
code() { curl -s -o /dev/null -w '%{http_code}' --noproxy '*' "$@"; }
BAD="${URL%/*}/wrong"
if [ "$(code -X POST --data "$BODY" "$BAD")" = 404 ]; then ok "a wrong secret answers 404"; else bad "wrong secret"; fi
if [ "$(code -X POST --data "$BODY" "$URL")" = 401 ]; then ok "a missing signature answers 401"; else bad "missing signature"; fi
SIG="sha256=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" | sed 's/^.* //')"
if [ "$(code -X POST -H "X-Hub-Signature-256: $SIG" --data "$BODY" "$URL")" = 202 ]; then ok "a signed request answers 202"; else bad "signed request"; fi
if [ "$(code -X POST -H "X-Hub-Signature-256: $SIG" --data-binary @<(head -c 1100000 /dev/zero | tr '\0' x) "$URL")" = 413 ]; then ok "a body over 1 MB answers 413"; else bad "big body"; fi
sleep 3
kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null; SERVE_PID=""
run "$Z" workflow runs hook
if has "hook" && has "completed"; then ok "the webhook started a run and the host finished it"; else bad "webhook run" "$OUT"; fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
