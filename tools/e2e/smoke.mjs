// Starts the built app on a throwaway data folder and waits for it to say it has loaded (`ZORIX_SMOKE`, see `app_ready`). It needs no driver,
// so it runs on every system, macOS included, where WebKit has no WebDriver. Usage: node tools/e2e/smoke.mjs PATH-TO-THE-APP [seconds]
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [application, seconds = "90"] = process.argv.slice(2);
if (!application) {
  console.error("Usage: node tools/e2e/smoke.mjs PATH-TO-THE-APP [seconds]");
  process.exit(2);
}
const home = mkdtempSync(join(tmpdir(), "zorix-smoke-"));
const child = spawn(application, [], { env: { ...process.env, ZORIX_HOME: home, ZORIX_SMOKE: "1" }, stdio: ["ignore", "pipe", "pipe"] });
let output = "";
let ready = false;
child.stdout.on("data", (chunk) => {
  output += chunk;
  if (!ready && /ZORIX ready /.test(output)) {
    // The app says it has loaded and closes itself; do not depend on how quickly it does, so a slow exit is not a failure.
    ready = true;
    clearTimeout(timer);
    console.log(output.split("\n").find((l) => l.startsWith("ZORIX ready")));
    setTimeout(() => child.kill(), 3000).unref();
    child.on("exit", () => {
      rmSync(home, { recursive: true, force: true });
      process.exit(0);
    });
  }
});
child.stderr.on("data", (chunk) => (output += chunk));

const failed = (why) => {
  console.error(`The app did not start: ${why}\n--- what it printed ---\n${output.slice(-2000)}`);
  child.kill();
  rmSync(home, { recursive: true, force: true });
  process.exit(1);
};
const timer = setTimeout(() => failed(`no "ZORIX ready" within ${seconds} s`), Number(seconds) * 1000);
child.on("exit", (code) => {
  if (!ready) failed(`it closed (code ${code}) without saying it was ready`);
});
