#!/usr/bin/env bash
# Checks of projects through the real `zorix` command line in a throwaway data folder. Nothing here uses your accounts, your
# keychain or your data folder, and every model is the built-in demo. Run it with `tools/live-tests.sh projects`.
#
#   scripted   always: a task given to an agent runs when it moves to In progress and lands in Review with its answer as a
#              deliverable; a task that depends on another waits, and `zorix serve` starts it once the first is done; a cycle
#              is refused; a task with a person's name starts nothing; comments, links and deliverables are recorded in the
#              history; the manager can be asked; and the output is the same facts as JSON.
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

"$Z" agent add --template researcher >/dev/null 2>&1

echo "== a project, and a task that an agent runs"
run "$Z" project create ops "Q4 Operations" --manager researcher --due 2026-12-20 --cost-limit 1
if [ $RC -eq 0 ] && has "Made Q4 Operations (OPS)"; then ok "a project was made, its key in capitals"; else bad "create" "$OUT"; fi
run "$Z" project create OPS "Again"
if [ $RC -ne 0 ] && has "already used"; then ok "a key cannot be used twice"; else bad "duplicate key" "$OUT"; fi
run "$Z" task add OPS "Triage the incident backlog" --assign researcher --priority urgent --due 2026-11-28
if [ $RC -eq 0 ] && has "Added OPS-1"; then ok "the first task is OPS-1"; else bad "add" "$OUT"; fi
run "$Z" task move OPS-1 in-progress --model demo
if [ $RC -eq 0 ] && has "needs you: review"; then ok "moving it to In progress ran it, and it waits in Review for you"; else bad "run" "$OUT"; fi
run "$Z" task show OPS-1 --full
if has "Review" && has "Deliverables" && has "Finished"; then ok "its answer is a deliverable and its run is on the record"; else bad "show" "$OUT"; fi
run "$Z" task list --needs-you
if has "OPS-1"; then ok "it is listed as needing you"; else bad "needs you" "$OUT"; fi

echo "== a person's task, a dependency and a cycle"
run "$Z" task add OPS "Decide the retention policy" --assign you
"$Z" task move OPS-2 in-progress >/dev/null 2>&1
run "$Z" task show OPS-2
if has "In progress" && ! has "Runs"; then ok "a task you have starts no run"; else bad "person task" "$OUT"; fi
run "$Z" task add OPS "Write the runbook" --assign researcher --after OPS-2
run "$Z" task move OPS-3 in-progress --model demo
if [ $RC -eq 0 ] && has "waits for OPS-2"; then ok "a task that depends on another waits"; else bad "blocked" "$OUT"; fi
run "$Z" task edit OPS-2 --after OPS-3
if [ $RC -ne 0 ] && has "wait for itself"; then ok "a cycle is refused"; else bad "cycle" "$OUT"; fi

echo "== serve starts a task when what it waited for is done"
"$Z" task move OPS-2 done >/dev/null 2>&1
"$Z" serve --model demo >"$WORK/serve.log" 2>&1 &
SERVE_PID=$!
sleep 4
kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null; SERVE_PID=""
run "$Z" task show OPS-3
if has "Review"; then ok "serve ran OPS-3 once OPS-2 was done, and it is in Review"; else bad "serve" "$OUT"; fi

echo "== notes, links, deliverables and history"
"$Z" task comment OPS-1 "Three look like duplicates." >/dev/null 2>&1
"$Z" task link OPS-1 https://github.com/acme/api/pull/482 >/dev/null 2>&1
printf '# Notes\nbody\n' > "$WORK/notes.md"
run "$Z" task deliver OPS-1 --file "$WORK/notes.md"
if [ $RC -eq 0 ]; then ok "a file became a deliverable"; else bad "deliver" "$OUT"; fi
run "$Z" task deliver OPS-1 --text a --url https://example.com
if [ $RC -ne 0 ] && has "exactly one"; then ok "two kinds of deliverable at once are refused"; else bad "deliver two" "$OUT"; fi
run "$Z" task history OPS-1
if has "commented on OPS-1" && has "linked https://github.com" && has "added a deliverable"; then ok "the history says what happened"; else bad "history" "$OUT"; fi

echo "== the manager and JSON"
run "$Z" project ask OPS "What is late?" --model demo
if [ $RC -eq 0 ] && has "Asked the manager of OPS"; then ok "the manager was asked and its run followed"; else bad "ask" "$OUT"; fi
run "$Z" project show OPS
if [ $RC -eq 0 ] && has "OPS-1" && has "Backlog (0)"; then ok "the project lists its tasks by lane"; else bad "show project" "$OUT"; fi
if "$Z" --json task list OPS 2>/dev/null | python3 -c 'import json,sys; v=json.loads(sys.stdin.read()); assert len(v)==3 and v[0]["task"]["key"].startswith("OPS-")'; then ok "--json prints the same tasks"; else bad "json"; fi
run "$Z" project archive OPS
run "$Z" project list
if has "No projects"; then ok "an archived project leaves the list"; else bad "archive" "$OUT"; fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
