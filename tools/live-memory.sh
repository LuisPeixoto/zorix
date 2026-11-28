#!/usr/bin/env bash
# The Phase 11 features, through the real command line on a throwaway data folder: nothing of yours is touched.
#
#   tools/live-memory.sh            all of it (ocr, watch, attach, telemetry)
#   tools/live-memory.sh ocr        scanned PDFs: downloads the two real models (about 12 MB) from their real address, checks them, reads a
#                                   picture of text and finds the words (set ZORIX_OCR_OFFLINE=1 to skip the download)
#   tools/live-memory.sh watch      a watched folder is read again by itself after a file changes
#   tools/live-memory.sh attach     a file is given to a run, a private one is refused, and (only if Ollama already runs, never started
#                                   here) a real local model answers from the file
#   tools/live-memory.sh telemetry  a run's spans reach a collector on this computer and carry no prompt
#
# It needs python3 and a built command line (it builds one). It never starts or stops Ollama.
set -euo pipefail
cd "$(dirname "$0")/.."

which="${1:-all}"
cargo build -q -p zorix-cli
ZORIX="$PWD/target/debug/zorix"
WORK="$(mktemp -d)"
HOME_DIR="$WORK/home"
PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; done
  rm -rf "$WORK"
}
trap cleanup EXIT
z() { "$ZORIX" --home "$HOME_DIR" "$@"; }
say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

# A PDF of one page that is a picture, the way a scanner makes it (no text layer).
make_scan() { # <jpeg> <width> <height> <out.pdf>
  python3 -I - "$1" "$2" "$3" "$4" <<'PY'
import sys
jpeg=open(sys.argv[1],'rb').read(); w,h=int(sys.argv[2]),int(sys.argv[3])
objs=[b"<< /Type /Catalog /Pages 2 0 R >>",
      b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
      b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 85] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>",
      None, None]
content=b"q 400 0 0 85 0 0 cm /Im0 Do Q"
out=bytearray(b"%PDF-1.4\n"); offs=[]
def put(n,body):
    offs.append(len(out)); out.extend(f"{n} 0 obj\n".encode()+body+b"\nendobj\n")
put(1,objs[0]); put(2,objs[1]); put(3,objs[2])
put(4,b"<< /Length %d >>\nstream\n"%len(content)+content+b"\nendstream")
put(5,(b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length %d >>\nstream\n"%(w,h,len(jpeg)))+jpeg+b"\nendstream")
x=len(out); out.extend(f"xref\n0 {len(offs)+1}\n0000000000 65535 f \n".encode())
for o in offs: out.extend(f"{o:010d} 00000 n \n".encode())
out.extend(f"trailer\n<< /Size {len(offs)+1} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n".encode())
open(sys.argv[4],'wb').write(out)
PY
}

ocr() {
  say "reading scans"
  if [ "${ZORIX_OCR_OFFLINE:-}" = 1 ]; then echo "skipped (ZORIX_OCR_OFFLINE=1)"; return; fi
  mkdir -p "$WORK/scans"
  make_scan crates/zorix-memory/tests/fixtures/scan-page.jpg 800 170 "$WORK/scans/invoice.pdf"
  z knowledge add "$WORK/scans" >/dev/null 2>"$WORK/add.txt" || { cat "$WORK/add.txt"; fail "knowledge add"; }
  z knowledge files "$WORK/scans" --failed | tee "$WORK/files.txt"
  grep -q "it is a scan" "$WORK/files.txt" || fail "a scan should be left out with its reason while reading scans is off"
  z settings set knowledge.ocr on 2>&1 | tee "$WORK/on.txt"
  grep -q "OCR models are ready" "$WORK/on.txt" || fail "the models were not downloaded and checked"
  ls -l "$HOME_DIR/models/ocr"
  z knowledge sync 2>&1 | tail -2
  z knowledge files "$WORK/scans" --failed | grep -q . && fail "the scan is still left out after reading scans was turned on"
  hit="$(z knowledge search "invoice thirty days" 2>&1 || true)"
  echo "$hit" | head -5
  echo "$hit" | grep -qi "thirty days" || fail "the words of the scan were not found"
  echo "$hit" | grep -q "invoice.pdf" || fail "the citation does not name the file"
  # A model file that is not the one fixed is never loaded.
  printf 'tampered' >> "$HOME_DIR/models/ocr/text-detection.onnx"
  # (the output is captured first: `grep -q` stops reading at the first match, and with pipefail the writer's SIGPIPE would count as a failure)
  refused="$(z knowledge sync 2>&1)"
  grep -q "its models are not on this computer" <<<"$refused" || { echo "$refused"; fail "a changed model file was used"; }
  z settings set knowledge.ocr off >/dev/null
  echo "ok: downloaded, checked, read, found, and a tampered model was refused"
}

watch() {
  say "watching a folder"
  mkdir -p "$WORK/watched"
  echo "The platform team deploys on Thursdays." > "$WORK/watched/handbook.md"
  z knowledge add "$WORK/watched" --watch >/dev/null 2>&1
  # The program itself is started in the background (not the shell function that wraps it), so that the trap kills the process and not just a subshell.
  "$ZORIX" --home "$HOME_DIR" knowledge watch >"$WORK/watch.out" 2>&1 &
  PIDS+=("$!"); disown "$!"
  sleep 4
  echo "The zebracorn migration happens on Friday." > "$WORK/watched/second.md"
  for _ in $(seq 1 40); do
    if z knowledge search zebracorn 2>/dev/null | grep -q zebracorn; then echo "ok: found a minute or less after the file appeared"; return; fi
    sleep 1
  done
  cat "$WORK/watch.out"; fail "the new file was not read by the watcher"
}

attach() {
  say "attaching files"
  mkdir -p "$WORK/attach"
  printf '# Launch\nThe codeword for the launch is MARMALADE-42.\nKey: sk-abcdefghijklmnopqrstuvwxyz0123456789\n' > "$WORK/attach/plan.md"
  z run "What is the codeword?" --model demo --attach "$WORK/attach/plan.md" 2>&1 | tail -3
  id="$(z list --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
  z show "$id" --json | python3 -I -c '
import json,sys
d=json.load(sys.stdin)["execution"]["request"]["attachments"][0]
assert d["name"]=="plan.md", d
assert "MARMALADE-42" in d["text"], "the text was not kept"
assert "sk-abcdef" not in d["text"], "a secret was kept"
print("ok: kept with the run, secret removed")'
  printf 'TOKEN=1\n' > "$WORK/attach/.env"
  if z run "Read it" --model demo --attach "$WORK/attach/.env" 2>"$WORK/refused.txt"; then fail ".env was attached"; fi
  grep -q "cannot be attached" "$WORK/refused.txt" || fail "the refusal does not say why"
  echo "ok: a private name is refused: $(head -1 "$WORK/refused.txt")"
  if curl -fsS -m 3 "${OLLAMA_HOST:-http://localhost:11434}/api/tags" >/dev/null 2>&1; then
    model="${ZORIX_LIVE_OLLAMA_MODEL:-qwen3:1.7b}"
    if curl -fsS -m 3 "${OLLAMA_HOST:-http://localhost:11434}/api/tags" | grep -q "\"${model%%:*}"; then
      answer="$(z run "What is the codeword in the attached file? Answer with the codeword only." --attach "$WORK/attach/plan.md" --model "ollama/$model" 2>/dev/null || true)"
      echo "ollama said: $answer"
      echo "$answer" | grep -q "MARMALADE-42" && echo "ok: a real local model answered from the file" || echo "note: the model did not repeat the codeword (a small model may not)"
    else
      echo "note: Ollama runs but $model is not pulled; skipping the real-model check"
    fi
  else
    echo "note: Ollama is not running here; skipping the real-model check (it is never started by this script)"
  fi
}

telemetry() {
  say "telemetry"
  python3 -I - "$WORK/collector.out" "$WORK/collector.port" <<'PY' &
import http.server, sys
out, portfile = sys.argv[1], sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("content-length", 0)); body = self.rfile.read(n)
        with open(out, "ab") as f:
            f.write(("POST %s %s %d\n" % (self.path, self.headers.get("content-type"), len(body))).encode()); f.write(body); f.write(b"\n--END--\n")
        self.send_response(200); self.send_header("content-type", "application/x-protobuf"); self.send_header("content-length", "0"); self.end_headers()
    def log_message(self, *a): pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
open(portfile, "w").write(str(s.server_address[1]))
s.serve_forever()
PY
  PIDS+=("$!"); disown "$!"
  for _ in $(seq 1 20); do [ -s "$WORK/collector.port" ] && break; sleep 0.25; done
  port="$(cat "$WORK/collector.port")"
  z settings set telemetry.endpoint "http://127.0.0.1:$port" 2>&1 | tail -1
  secret_prompt="ZEBRACORN-SECRET-PROMPT-$$"
  z run "$secret_prompt" --model demo >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do grep -q "^POST /v1/traces" "$WORK/collector.out" 2>/dev/null && break; sleep 0.5; done
  grep -q "^POST /v1/traces application/x-protobuf" "$WORK/collector.out" || fail "the collector received no spans"
  grep -aq "invoke_agent" "$WORK/collector.out" || fail "the spans are not the ones expected (no invoke_agent)"
  if grep -aq "$secret_prompt" "$WORK/collector.out"; then fail "the prompt leaked into the export"; fi
  z settings set telemetry.endpoint off >/dev/null 2>&1 || true
  echo "ok: spans arrived at the collector and the prompt did not"
}

case "$which" in
  ocr) ocr ;;
  watch) watch ;;
  attach) attach ;;
  telemetry) telemetry ;;
  all) attach; watch; telemetry; ocr ;;
  *) fail "unknown section: $which (ocr, watch, attach, telemetry)" ;;
esac
say "done"
