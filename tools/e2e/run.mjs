// Runs the end-to-end scenarios against the built app through tauri-driver (Linux and Windows; WebKit has no driver on macOS, where
// `smoke.mjs` is the check). Usage: node tools/e2e/run.mjs PATH-TO-THE-APP [PATH-TO-NATIVE-DRIVER]
// A failing scenario leaves a screenshot and the page's source in `e2e-artifacts/`.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { scenarios } from "./scenarios.mjs";
import { Session, sleep } from "./webdriver.mjs";

const [appArgument, driverArgument] = process.argv.slice(2);
// The native driver launches the app itself, from its own folder, so both paths are made absolute (a relative one with `/` failed on Windows).
const application = appArgument && resolve(appArgument);
const nativeDriver = driverArgument && resolve(driverArgument);
if (!application) {
  console.error("Usage: node tools/e2e/run.mjs PATH-TO-THE-APP [PATH-TO-NATIVE-DRIVER]");
  process.exit(2);
}
const PORT = 4444;
const base = `http://127.0.0.1:${PORT}`;
const artifacts = join(process.cwd(), "e2e-artifacts");
mkdirSync(artifacts, { recursive: true });
const slug = (text) => text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 60);

let failures = 0;
for (const scenario of scenarios) {
  const home = mkdtempSync(join(tmpdir(), "zorix-e2e-"));
  // The driver starts the app, so the app inherits the driver's environment: a throwaway data folder, never the person's own.
  const driver = spawn("tauri-driver", [...(nativeDriver ? ["--native-driver", nativeDriver] : []), "--port", String(PORT)], {
    env: { ...process.env, ZORIX_HOME: home },
    stdio: ["ignore", "inherit", "inherit"],
  });
  let session;
  try {
    for (let i = 0; i < 40; i++) {
      try {
        await fetch(`${base}/status`);
        break;
      } catch {
        await sleep(250);
      }
    }
    session = await Session.start(base, application);
    await scenario.run(session);
    console.log(`ok    ${scenario.name}`);
  } catch (error) {
    failures++;
    console.error(`FAIL  ${scenario.name}\n      ${error.message}`);
    if (session) {
      try {
        writeFileSync(join(artifacts, `${slug(scenario.name)}.png`), await session.screenshot());
        writeFileSync(join(artifacts, `${slug(scenario.name)}.html`), await session.source());
      } catch (inner) {
        console.error(`      (no screenshot: ${inner.message})`);
      }
    }
  } finally {
    await session?.end();
    driver.kill();
    await sleep(500);
    rmSync(home, { recursive: true, force: true });
  }
}
console.log(failures === 0 ? `\nAll ${scenarios.length} scenarios passed.` : `\n${failures} of ${scenarios.length} scenarios failed; see ${artifacts}.`);
process.exit(failures === 0 ? 0 : 1);
