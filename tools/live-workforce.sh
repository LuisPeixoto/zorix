#!/usr/bin/env bash
# Checks of the workforce through the real `zorix` command line in a throwaway data folder. Nothing here uses your accounts, your
# keychain or your data folder, and every model is the built-in demo. Run it with `tools/live-tests.sh workforce`.
#
#   scripted   always: agents become workers in departments under a chain that ends at you; a worker cannot be freer or spend more
#              than the one it reports to; levels and budgets are stored and shown; a manager's plan and report are asked for and
#              kept; the schedule is off until turned on; a task run by a worker runs and lands in Review; the organisation's
#              settings and departments change and refuse what does not fit; and the output is the same facts as JSON.
set -uo pipefail
cd "$(dirname "$0")/.."

cargo build -q -p zorix-cli || { echo "could not build zorix" >&2; exit 1; }
Z="$PWD/target/debug/zorix"
WORK="$(mktemp -d)"
export ZORIX_HOME="$WORK/home" ZORIX_SECRETS=env ZORIX_TEST_SECRETS_FILE="$WORK/secrets.json"
trap 'rm -rf "${WORK:?}"' EXIT

pass=0; fail=0
ok()  { echo "  ok    $1"; pass=$((pass + 1)); }
bad() { echo "  FAIL  $1"; fail=$((fail + 1)); [ -n "${2:-}" ] && echo "$2" | sed 's/^/        /' | head -30; }
run() { OUT="$("$@" 2>&1)"; RC=$?; }
has() { echo "$OUT" | grep -q -- "$1"; }

"$Z" agent add --template researcher >/dev/null 2>&1
"$Z" agent add --template writer >/dev/null 2>&1

echo "== hiring, and a chain that ends at you"
run "$Z" worker hire researcher --role "Research lead" --dept research --manager --level budgeted --budget 60 --task-time 120
if [ $RC -eq 0 ] && has "researcher was hired as Research lead in research, at Budgeted"; then ok "an agent was hired as a manager at Budgeted"; else bad "hire" "$OUT"; fi
run "$Z" worker hire researcher --role "Again" --dept research
if [ $RC -ne 0 ] && has "already a worker"; then ok "an agent is one worker at most"; else bad "twice" "$OUT"; fi
run "$Z" worker hire writer --role Writer --dept operations --reports-to researcher --level full --budget 10
if [ $RC -ne 0 ] && has "freer than researcher"; then ok "a worker cannot be freer than who it reports to"; else bad "freer" "$OUT"; fi
run "$Z" worker hire writer --role Writer --dept operations --reports-to researcher --level notify --budget 100
if [ $RC -ne 0 ] && has "larger weekly budget"; then ok "a budget cannot be larger than the manager's"; else bad "budget" "$OUT"; fi
run "$Z" worker hire writer --role Writer --dept sales --reports-to researcher --level notify --budget 10
if [ $RC -ne 0 ] && has "does not exist"; then ok "a department must exist"; else bad "department" "$OUT"; fi
run "$Z" worker hire writer --role Writer --dept operations --reports-to researcher --level notify --budget 10 --still-ask repo:acme/web:write
if [ $RC -eq 0 ]; then ok "a worker was hired under the manager, with what always asks"; else bad "hire writer" "$OUT"; fi
run "$Z" worker list
if has "You" && has "researcher" && has "writer" && has "Notify"; then ok "the chart shows the chain from you down"; else bad "list" "$OUT"; fi

echo "== levels, limits and what cannot be done"
run "$Z" worker level researcher ask
if [ $RC -ne 0 ] && has "freer than researcher"; then ok "a manager cannot be lowered below its reports"; else bad "lower manager" "$OUT"; fi
run "$Z" worker fire researcher
if [ $RC -ne 0 ] && has "has reports (writer)"; then ok "somebody with reports is not let go"; else bad "fire" "$OUT"; fi
run "$Z" worker level writer suggest
if [ $RC -eq 0 ] && has "writer is at Suggest"; then ok "a level was changed"; else bad "level" "$OUT"; fi
run "$Z" worker show writer
if has "Suggest" && has "reports to researcher" && has "repo:acme/web:write" && has '\$10.00 a week'; then ok "the worker shows its level and its limits"; else bad "show" "$OUT"; fi
run "$Z" worker level writer loose
if [ $RC -ne 0 ] && has "is not a level"; then ok "a level has to be one of the five"; else bad "bad level" "$OUT"; fi

echo "== a task run by a worker, and the manager's week"
"$Z" project create OPS "Q4 Operations" --manager researcher >/dev/null 2>&1
"$Z" worker level writer notify >/dev/null 2>&1
"$Z" task add OPS "Write the incident summary" --assign writer >/dev/null 2>&1
run "$Z" task move OPS-1 in-progress --model demo
if [ $RC -eq 0 ] && has "needs you: review"; then ok "a worker's task ran under its level and waits in Review"; else bad "task" "$OUT"; fi
run "$Z" worker plan researcher "Focus on the migration." --model demo
if [ $RC -eq 0 ] && has "Asked researcher to plan the week"; then ok "the manager was asked to plan the week"; else bad "plan" "$OUT"; fi
run "$Z" worker report researcher --model demo
if [ $RC -eq 0 ] && has "Asked researcher to write the report"; then ok "the manager was asked for the report"; else bad "report" "$OUT"; fi
run "$Z" worker show researcher
if has "Latest" ; then ok "the answer was kept as the latest plan or report"; else bad "kept" "$OUT"; fi
run "$Z" worker plan writer
if [ $RC -ne 0 ] && has "not a manager"; then ok "only a manager plans"; else bad "not a manager" "$OUT"; fi
run "$Z" worker schedule researcher on
if [ $RC -eq 0 ] && has "plans Mondays 09:00 and reports Fridays 16:00"; then ok "the schedule was turned on"; else bad "schedule on" "$OUT"; fi
run "$Z" worker schedule researcher off
if [ $RC -eq 0 ] && has "has no schedule"; then ok "and off again"; else bad "schedule off" "$OUT"; fi
run "$Z" worker schedule writer on
if [ $RC -ne 0 ] && has "not a manager"; then ok "a worker that is not a manager has no schedule"; else bad "schedule writer" "$OUT"; fi

echo "== the organisation: settings, departments, activity and JSON"
run "$Z" org settings --budget 120 --level ask
if [ $RC -eq 0 ] && has '\$120.00' && has "start at Ask"; then ok "the weekly budget and the level of new workers were set"; else bad "settings" "$OUT"; fi
run "$Z" org dept remove operations
if [ $RC -ne 0 ] && has "does not exist"; then ok "a department with workers cannot be removed"; else bad "dept remove" "$OUT"; fi
run "$Z" org dept add sales Sales --description "Selling things" --lead writer
if [ $RC -eq 0 ]; then ok "a department was added with a lead"; else bad "dept add" "$OUT"; fi
run "$Z" org dept remove sales
if [ $RC -eq 0 ]; then ok "an empty department was removed"; else bad "dept remove empty" "$OUT"; fi
run "$Z" org show
if [ $RC -eq 0 ] && has "Workforce" && has "spend this week" && has "of \$120.00"; then ok "the organisation shows its numbers and chart"; else bad "show" "$OUT"; fi
run "$Z" org activity
if [ $RC -eq 0 ] && has "was hired" && has "Level changed"; then ok "activity lists what happened"; else bad "activity" "$OUT"; fi
if "$Z" --json org show 2>/dev/null | python3 -c 'import json,sys; v=json.loads(sys.stdin.read()); assert len(v["workers"])==2 and v["budget_micros"]==120000000'; then ok "--json prints the same organisation"; else bad "json"; fi
run "$Z" worker fire writer
if [ $RC -eq 0 ]; then ok "a worker without reports was let go"; else bad "fire writer" "$OUT"; fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
