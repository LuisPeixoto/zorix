// Turns the JSON of `cargo llvm-cov --json --summary-only` into a table per crate: lines, functions and regions covered by the tests.
// Usage: node tools/coverage-summary.mjs cov.json [--markdown]
import { readFileSync } from "node:fs";

const [file, flag] = process.argv.slice(2);
if (!file) {
  console.error("Usage: node tools/coverage-summary.mjs cov.json [--markdown]");
  process.exit(2);
}
const data = JSON.parse(readFileSync(file, "utf8")).data[0];
const crates = new Map();
for (const entry of data.files) {
  const match = entry.filename.match(/\/(?:crates\/([^/]+)|apps\/desktop\/(src-tauri))\//);
  const name = match ? (match[1] ?? "zorix-desktop") : "other";
  const sum = crates.get(name) ?? { lines: { count: 0, covered: 0 }, functions: { count: 0, covered: 0 }, regions: { count: 0, covered: 0 } };
  for (const kind of ["lines", "functions", "regions"]) {
    sum[kind].count += entry.summary[kind].count;
    sum[kind].covered += entry.summary[kind].covered;
  }
  crates.set(name, sum);
}
const pct = (s) => (s.count === 0 ? "—" : `${((100 * s.covered) / s.count).toFixed(1)} %`);
const rows = [...crates.entries()].sort((a, b) => a[0].localeCompare(b[0]));
const total = { lines: data.totals.lines, functions: data.totals.functions, regions: data.totals.regions };
if (flag === "--markdown") {
  console.log("| Crate | Lines | Functions | Regions |\n|---|---|---|---|");
  for (const [name, s] of rows) console.log(`| \`${name}\` | ${pct(s.lines)} (${s.lines.covered}/${s.lines.count}) | ${pct(s.functions)} | ${pct(s.regions)} |`);
  console.log(`| **all** | **${pct(total.lines)}** (${total.lines.covered}/${total.lines.count}) | **${pct(total.functions)}** | **${pct(total.regions)}** |`);
} else {
  const width = Math.max(...rows.map(([n]) => n.length), 5);
  for (const [name, s] of rows) console.log(`${name.padEnd(width)}  lines ${pct(s.lines).padStart(8)}  functions ${pct(s.functions).padStart(8)}  regions ${pct(s.regions).padStart(8)}`);
  console.log(`${"all".padEnd(width)}  lines ${pct(total.lines).padStart(8)}  functions ${pct(total.functions).padStart(8)}  regions ${pct(total.regions).padStart(8)}`);
}
