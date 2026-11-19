#!/usr/bin/env bash
# Live checks of the integrations (MCP servers, native tools, the sandbox) against real servers, through the real `zorix`
# command line in a throwaway data folder. Nothing here uses your accounts or your keychain: only servers that need no
# sign-in, and public data. Run it with `tools/live-tests.sh integrations`.
#
#   DeepWiki and Cloudflare docs  open remote MCP servers (HTTPS, no sign-in)
#   @modelcontextprotocol/server-everything and -filesystem   stdio servers, when Node and npx are installed
#   GitHub                        a public read with no token
#   Python                        downloaded and checked, then a program is run in the sandbox
#
# Remote servers that need OAuth, and Jira, Slack, Discord and Postgres, are only tried with connections you have stored for
# ZORIX yourself (`zorix connect add …`, `zorix mcp login …`); this script never makes one for you.
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
# Run a command and keep stdout+stderr in $OUT; the exit code is in $RC.
run() { OUT="$("$@" 2>&1)"; RC=$?; }

# What the newest tool call returned, from the record the run left (the model's last words say nothing about it).
last_result() {
  python3 - "$ZORIX_HOME/zorix.sqlite" <<'PY'
import json, sqlite3, sys
row = sqlite3.connect(sys.argv[1]).execute("select doc from tool_calls order by rowid desc limit 1").fetchone()
print(json.dumps(json.loads(row[0]).get("result")) if row else "")
PY
}

script() { printf '%s' "$2" > "$WORK/$1.json"; echo "script:$WORK/$1.json"; }

# A remote server with no sign-in: add, turn on, see its tools, approve them, then let a scripted run call one.
remote() {
  local name="$1" id="$2" tool="$3" args="$4" expect="$5"
  echo "== $name (remote, no sign-in)"
  run "$Z" mcp add --catalog "$id" "$name"
  [ $RC -eq 0 ] && ok "added from the catalogue" || { bad "add" "$OUT"; return; }
  run "$Z" mcp enable "$name" --yes
  run "$Z" mcp tools "$name"
  if [ $RC -eq 0 ] && echo "$OUT" | grep -q "new"; then ok "started and listed its tools ($(echo "$OUT" | grep -c ' new ') new)"; else bad "tools" "$OUT"; return; fi
  run "$Z" mcp review "$name" --approve-all
  [ $RC -eq 0 ] && ok "approved after review" || { bad "approve" "$OUT"; return; }
  local model; model="$(script "$name" "[{\"call\": {\"calls\": [{\"tool\": \"mcp.$name.$tool\", \"arguments\": $args}]}}, {\"say\": \"Done.\"}]")"
  run "$Z" run "use $name" --model "$model" --allow "mcp.$name.*" --approve yes
  local id_run; id_run="$("$Z" list --json 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
  if [ $RC -eq 0 ] && last_result | grep -qi "$expect"; then ok "a run called $tool and got an answer containing \"${expect}\""; else bad "run" "$OUT"; fi
  run "$Z" events "$id_run"
  echo "$OUT" | grep -q "tool.completed" && ok "the record shows the call completed" || bad "events" "$OUT"
}

remote deepwiki deepwiki read_wiki_structure '{"repoName": "rust-lang/rust"}' "rust"
remote cfdocs cloudflare-docs search_cloudflare_documentation '{"query": "workers kv"}' "workers"

echo "== stdio servers"
if command -v npx >/dev/null 2>&1; then
  mkdir -p "$WORK/files"; FOLDER="$(cd "$WORK/files" && pwd -P)"; echo "hello from the live test" > "$FOLDER/note.txt"
  run "$Z" mcp add everything --command npx --arg -y --arg @modelcontextprotocol/server-everything
  run "$Z" mcp enable everything --yes
  run "$Z" mcp tools everything
  if [ $RC -eq 0 ] && echo "$OUT" | grep -q "echo"; then
    ok "server-everything started through npx and listed its tools"
    run "$Z" mcp review everything --approve-all
    model="$(script everything '[{"call": {"calls": [{"tool": "mcp.everything.echo", "arguments": {"message": "ping from zorix"}}]}}, {"say": "Done."}]')"
    run "$Z" run "echo" --model "$model" --allow "mcp.everything.*" --approve yes
    last_result | grep -q "ping from zorix" && ok "echo came back through the server" || bad "echo" "$OUT"
  else
    bad "server-everything" "$OUT"
  fi
  run "$Z" mcp add --catalog filesystem files --input "folder=$FOLDER"
  run "$Z" mcp enable files --yes
  run "$Z" mcp tools files
  if [ $RC -eq 0 ] && echo "$OUT" | grep -q "read"; then
    ok "server-filesystem listed its tools for one folder"
    run "$Z" mcp review files --approve-all
    model="$(script files "[{\"call\": {\"calls\": [{\"tool\": \"mcp.files.read_text_file\", \"arguments\": {\"path\": \"$FOLDER/note.txt\"}}]}}, {\"say\": \"Done.\"}]")"
    run "$Z" run "read" --model "$model" --allow "mcp.files.*" --approve yes
    last_result | grep -q "hello from the live test" && ok "a file was read through the server" || bad "read" "$OUT"
    model="$(script outside "[{\"call\": {\"calls\": [{\"tool\": \"mcp.files.read_text_file\", \"arguments\": {\"path\": \"/etc/hosts\"}}]}}, {\"say\": \"Done.\"}]")"
    run "$Z" run "read outside" --model "$model" --allow "mcp.files.*" --approve yes
    echo "$OUT" | grep -qi "tool.failed.*access denied" && ok "the server itself refused a path outside its folder" || bad "outside" "$OUT"
  else
    bad "server-filesystem" "$OUT"
  fi
else
  skip "Node and npx are not installed, so the stdio servers are skipped"
fi

echo "== GitHub, public data, no token"
run "$Z" connect add github pub --no-token
run "$Z" connect test pub
[ $RC -eq 0 ] && ok "$(echo "$OUT" | head -1)" || bad "connect test" "$OUT"
model="$(script git '[{"call": {"calls": [{"tool": "github.get_issue", "arguments": {"repo": "rust-lang/rust", "number": 1}}]}}, {"say": "Done."}]')"
run "$Z" run "read an issue" --model "$model" --allow-repo rust-lang/rust --approve never
if [ $RC -eq 0 ]; then
  run "$Z" events "$("$Z" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
  last_result | grep -q "title" && ok "a public issue was read without a token" || bad "events" "$OUT"
else
  bad "run" "$OUT"
fi
model="$(script gitwrite '[{"call": {"calls": [{"tool": "github.create_issue", "arguments": {"repo": "rust-lang/rust", "title": "x"}}]}}, {"say": "Done."}]')"
run "$Z" run "write" --model "$model" --allow-repo rust-lang/rust --approve never
[ $RC -ne 0 ] && ok "a write needs a person and a token, and stopped there (exit $RC)" || bad "a write was not stopped" "$OUT"

echo "== Python in the sandbox"
run "$Z" sandbox install python
if [ $RC -eq 0 ]; then
  ok "Python downloaded and matched its checksum"
  model="$(script py '[{"call": {"calls": [{"tool": "code.run", "arguments": {"language": "python", "code": "import sys\nprint(sum(range(101)), sys.version_info[:2])"}}]}}, {"say": "Done."}]')"
  run "$Z" run "py" --model "$model" --allow-code python --approve never
  run "$Z" events "$("$Z" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
  last_result | grep -q "5050" && ok "a Python program ran in the sandbox and printed 5050" || bad "python run" "$OUT"
else
  bad "python install" "$OUT"
fi

echo "== needs your own accounts"
skip "OAuth servers, Jira, Slack, Discord and Postgres: store a connection for ZORIX yourself and run its tests by hand"

echo
echo "$pass passed, $fail failed, $skipped skipped"
[ "$fail" -eq 0 ]
