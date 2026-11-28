/** What a person lets another MCP client do with ZORIX. */
export interface ServeOptions {
  runs: boolean;
  approvals: boolean;
  readRuns: boolean;
}

/** The command a client runs to use ZORIX. Answering approvals and reading every run exist only together with runs. */
export function serveCommand({ runs, approvals, readRuns }: ServeOptions): string {
  const flags = runs ? ["--allow-runs", ...(approvals ? ["--allow-approvals"] : []), ...(readRuns ? ["--allow-read-runs"] : [])] : [];
  return ["zorix", "mcp", "serve", ...flags].join(" ");
}
