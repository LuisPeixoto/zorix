// The only file that talks to the Rust side. Types mirror what the engine sends (serde JSON).
import { Channel, invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";

export type ExecutionState =
  | "created"
  | "queued"
  | "running"
  | "waiting_for_model"
  | "waiting_for_tool"
  | "waiting_for_approval"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cost_micros: number;
  model_calls: number;
  tool_calls: number;
}

export interface Execution {
  id: string;
  tenant: string;
  request: { prompt: string; instructions: string | null; model: string };
  state: ExecutionState;
  version: number;
  iteration: number;
  tool_calls_requested: number;
  active_ms: number;
  usage: Usage;
  conversation?: string | null;
  result: { text: string; structured: unknown } | null;
  failure: { kind: string; message: string; limit: string | null } | null;
  cancel_reason: string | null;
  created_at: number;
  updated_at: number;
  started_at: number | null;
  finished_at: number | null;
}

export interface ExecEvent {
  seq: number;
  execution_id: string;
  at: number;
  schema_version: number;
  kind: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  payload: Record<string, any>;
}

export interface Approval {
  id: string;
  execution_id: string;
  call_id: string;
  tool: string;
  arguments: unknown;
  call_hash: string;
  reason: string;
  status: "pending" | "approved" | "denied";
  requested_at: number;
  decided_at: number | null;
  decided_by: string | null;
  note: string | null;
}

export interface ToolCall {
  call_id: string;
  tool: string;
  arguments: unknown;
  status: "requested" | "awaiting_approval" | "started" | "succeeded" | "failed" | "denied";
  attempts: number;
  error: string | null;
}

export interface Summary {
  iterations: number;
  model_calls: number;
  model_retries: number;
  models: string[];
  fallbacks: number;
  total_ms: number | null;
  active_ms: number;
  usage: Usage;
  steps: { kind: string; name: string; provider: string | null; duration_ms: number }[];
}

export interface RunDetail {
  execution: Execution;
  tool_calls: ToolCall[];
  approvals: Approval[];
  summary: Summary;
}

export interface ToolSpec {
  id: string;
  name: string;
  description: string;
  requires_approval: boolean;
  annotations: { read_only: boolean; destructive: boolean; idempotent: boolean; open_world: boolean };
  /** Where it comes from: built in, memory, an MCP server, a native integration or the sandbox. */
  source?: { kind: "builtin" | "memory" | "mcp" | "integration" | "sandbox"; server?: string; connection?: string | null };
}

export interface AppInfo {
  version: string;
  data_dir: string;
  error: string | null;
  start_view?: string | null;
}

export interface StartRun {
  prompt: string;
  instructions?: string;
  allow_tools: string[];
  allow_read: string[];
  allow_write: string[];
  workspace?: string;
  max_iterations?: number;
  /** `provider/model`, `route:name`, an alias or `demo`. Left out: the default. */
  model?: string;
  /** Continue this conversation. Left out: a new one starts. */
  conversation?: string;
  /** What the run may use from memory. Left out: everything. */
  memory?: { conversation: boolean; long_term: boolean; knowledge: boolean; operations: boolean };
  /** The servers and services the run may reach. Each entry means what the matching `--allow-…` flag means. */
  integrations?: Partial<Record<"mcp" | "http" | "repo" | "jira" | "chat" | "db" | "code", string[]>>;
}

export interface MemoryRow {
  id: string;
  kind: "fact" | "preference";
  /** `global`, `project:<folder>` or `conversation:<id>`. */
  scope: string;
  scope_label: string;
  subject: string | null;
  content: string;
  status: "active" | "replaced" | "expired";
  replaced_by: string | null;
  by: string;
  proposed_in: string | null;
  created_at: number;
  confirmed_at: number;
  valid_until: number | null;
  stale_days: number | null;
  use_count: number;
}

export interface NewFact {
  content: string;
  kind: "fact" | "preference";
  scope: string;
  subject?: string;
  valid_until?: string;
}

export interface MemoryChange {
  id: string;
  content?: string;
  subject?: string;
  clear_subject?: boolean;
  valid_until?: string;
  clear_valid_until?: boolean;
  scope?: string;
  confirm?: boolean;
}

export interface RecallItem {
  id: string;
  kind: "fact" | "exchange" | "chunk" | "past_work";
  text: string;
  label: string;
  score: number;
  found_by: string[];
  tokens: number;
}

export interface RunContext {
  recall: {
    query: string;
    degraded: string | null;
    embedding_model: string | null;
    scrubbed: boolean;
    items: RecallItem[];
  } | null;
  segments: { origin: string; tokens: number; kept: boolean }[] | null;
  token_factor: number | null;
}

export interface ProviderRow {
  name: string;
  kind: string;
  url: string;
  local: boolean;
  enabled: boolean;
  key: "not_needed" | "present" | "missing" | "unavailable";
  key_note: string | null;
}

export interface ModelRow {
  reference: string;
  name: string;
  context_window: number;
  tools: boolean;
  vision: boolean;
  local: boolean;
  price_known: boolean;
}

export interface ModelChoices {
  aliases: string[];
  routes: string[];
  models: ModelRow[];
  /** What "default" means, or why it means nothing yet. */
  default: { Ok: string } | { Err: string };
}

export const listMemory = (includeInactive: boolean) => invoke<MemoryRow[]>("list_memory", { includeInactive });
export const addMemory = (fact: NewFact) => invoke<string>("add_memory", { fact });
export const updateMemory = (change: MemoryChange) => invoke<MemoryRow>("update_memory", { change });
export const memoryUsage = (id: string) => invoke<string[]>("memory_usage", { id });
export const forgetMemory = (id: string) => invoke<boolean>("forget_memory", { id });
export const runContext = (id: string) => invoke<RunContext>("run_context", { id });
export interface SyncCounts {
  scanned: number;
  indexed: number;
  unchanged: number;
  skipped: number;
  failed: number;
  removed: number;
  chunks: number;
  embedded: number;
  cancelled: boolean;
  degraded: string | null;
  too_many_files: boolean;
}

export interface RootRow {
  id: string;
  path: string;
  scope: string;
  enabled: boolean;
  auto_sync: boolean;
  include: string[];
  exclude: string[];
  last_sync_at: number | null;
  last_sync: SyncCounts | null;
  indexed: number;
  skipped: number;
  failed: number;
  syncing: boolean;
}

export interface NewRoot {
  path: string;
  project?: string;
  include?: string[];
  exclude?: string[];
  auto_sync?: boolean;
}

export interface FileRow {
  file: string;
  status: "indexed" | "failed" | "skipped";
  reason: string | null;
}

export interface Hit {
  title: string;
  text: string;
  keyword_rank: number | null;
  vector_rank: number | null;
  cosine: number | null;
  execution: string | null;
  state: string | null;
}

export interface Found {
  hits: Hit[];
  degraded: string | null;
}

export interface EmbeddingsState {
  model: string | null;
  dimensions: number | null;
  allow_cloud: boolean;
  embedded: number;
  texts: number;
}

export interface EmbeddingModel {
  reference: string;
  dimensions: number | null;
  local: boolean;
  installed: boolean;
}

export interface SyncProgress {
  root: string;
  progress: { phase: "scanning" | "indexing" | "embedding" | "done"; done: number; total: number; file: string | null; errors: number };
}

export interface SyncDone {
  root: string;
  report: SyncCounts | null;
  error: string | null;
}

export const listRoots = () => invoke<RootRow[]>("list_roots");
export const addRoot = (root: NewRoot) => invoke<RootRow>("add_root", { root });
export const syncRoot = (id: string) => invoke<boolean>("sync_root", { id });
export const cancelSync = (id: string) => invoke<void>("cancel_sync", { id });
export const removeRoot = (id: string) => invoke<boolean>("remove_root", { id });
export const rootFiles = (id: string, problemsOnly: boolean) => invoke<FileRow[]>("root_files", { id, problemsOnly });
export const tryQuery = (query: string, corpus: "knowledge" | "history", project?: string) =>
  invoke<Found>("try_query", { query, corpus, project });
export const embeddingsInfo = () => invoke<EmbeddingsState>("embeddings_info");
export const embeddingModels = () => invoke<[EmbeddingModel[], string[]]>("embedding_models");
export const setEmbeddings = (model: string, dimensions: number | undefined, cloud: boolean) =>
  invoke<number>("set_embeddings", { model, dimensions, cloud });
export const embeddingsOff = () => invoke<void>("embeddings_off");
export const testEmbeddings = () => invoke<[number, number]>("test_embeddings");
export const reembed = () => invoke<boolean>("reembed");
export const reembedding = () => invoke<boolean>("reembedding");

/** Follow the background jobs of the knowledge index. Returns a function that stops listening. */
export async function onKnowledge(handlers: {
  progress?: (p: SyncProgress) => void;
  done?: (d: SyncDone) => void;
  embedded?: (d: { facts: number; error: string | null }) => void;
}): Promise<() => void> {
  const stops: UnlistenFn[] = [];
  if (handlers.progress) stops.push(await listen<SyncProgress>("knowledge://progress", (e) => handlers.progress?.(e.payload)));
  if (handlers.done) stops.push(await listen<SyncDone>("knowledge://done", (e) => handlers.done?.(e.payload)));
  if (handlers.embedded) stops.push(await listen<{ facts: number; error: string | null }>("knowledge://embedded", (e) => handlers.embedded?.(e.payload)));
  return () => stops.forEach((stop) => stop());
}

export const listProviders = () => invoke<ProviderRow[]>("list_providers");
export const listModels = () => invoke<ModelChoices>("list_models");
/** The key goes straight to the system keychain on the Rust side; nothing keeps it here. */
export const setProviderKey = (name: string, key: string) => invoke<void>("set_provider_key", { name, key });
export const removeProviderKey = (name: string) => invoke<boolean>("remove_provider_key", { name });
export const testProvider = (name: string) => invoke<number>("test_provider", { name });
export const appInfo = () => invoke<AppInfo>("app_info");
export const listTools = () => invoke<ToolSpec[]>("list_tools");
export const listRuns = (limit = 50) => invoke<Execution[]>("list_runs", { limit });
export const runDetail = (id: string) => invoke<RunDetail>("run_detail", { id });
export const startRun = (request: StartRun) => invoke<Execution>("start_run", { request });
export const stopWatch = (id: string) => invoke<void>("stop_watch", { id });
export const pauseRun = (id: string) => invoke<Execution>("pause_run", { id });
export const resumeRun = (id: string) => invoke<Execution>("resume_run", { id });
export const cancelRun = (id: string, reason?: string) => invoke<Execution>("cancel_run", { id, reason });
export const decideRun = (
  id: string,
  approvalId: string,
  approve: boolean,
  callHash: string,
  note?: string,
) => invoke<Execution>("decide_run", { id, approvalId, approve, note, callHash });

/** Stored events after `afterSeq`, then live ones. The promise ends when the stream does. */
export function watchRun(id: string, afterSeq: number, onEvent: (event: ExecEvent) => void): Promise<void> {
  const channel = new Channel<ExecEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("watch_run", { id, afterSeq, onEvent: channel });
}

/** Turn whatever an invoke rejected with into a sentence. */
export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : JSON.stringify(error);
}

// ---------------------------------------------------------------------------------------------------------------------
// Integrations: MCP servers, connections and the sandbox

export type ServerState = "off" | "ready" | "starting" | "restarting" | "needs_login" | "failed" | "stopped";

export interface ServerRow {
  name: string;
  kind: "stdio" | "http";
  enabled: boolean;
  state: ServerState;
  reason: string | null;
  summary: string;
  era: string | null;
  server_name: string | null;
  server_version: string | null;
  tools_listed: number;
  tools_ready: number;
  tools_to_review: number;
  auth: "none" | "bearer" | "oauth" | null;
  signed_in: boolean;
  logo: string | null;
}

export interface McpToolRow {
  name: string;
  status: "new" | "pinned" | "changed" | "disabled";
  trust: "ask" | "read_only_no_ask";
  listed: boolean;
  title: string | null;
  description: string;
  approved_description: string | null;
  changed_fields: string[];
  inputs: string[];
  schema: unknown;
  approved_schema: unknown | null;
  read_only_hint: boolean;
  destructive_hint: boolean;
  open_world_hint: boolean;
  can_skip_asking: boolean;
}

export interface ServerDetail extends ServerRow {
  tools: McpToolRow[];
  log: string[];
  command: string | null;
  args: string[];
  /** Name, whether it is a secret (its value is never sent here), and its value when it is not. */
  env: [string, boolean, string | null][];
  cwd: string | null;
  url: string | null;
}

export interface CatalogEntry {
  id: string;
  name: string;
  summary: string;
  category: string;
  homepage: string;
  logo: string;
  default_name: string;
  transport:
    | { kind: "stdio"; command: string; args: string[]; env: { name: string; label: string; secret: boolean }[]; inputs: { name: string; label: string; kind: string }[] }
    | { kind: "http"; url: string; auth: "none" | "bearer" | "oauth" };
  requires?: string | null;
  token_help?: string | null;
  notes?: string | null;
}

export interface AddServer {
  name?: string;
  catalog?: string;
  inputs?: Record<string, string>;
  command?: string;
  args?: string[];
  env?: { name: string; value: string; secret: boolean }[];
  cwd?: string;
  url?: string;
  auth?: "none" | "bearer" | "oauth";
  client_id?: string;
  scopes?: string[];
  token?: string;
  secrets?: Record<string, string>;
}

export interface ImportPreview {
  source: string;
  path: string;
  rows: { original_name: string; name: string; summary: string | null; warnings: string[]; skipped: string | null; secrets: number }[];
}

export type ConnectionKind = "http" | "webhook" | "github" | "gitlab" | "jira" | "slack" | "discord" | "postgres" | "sqlite";

export interface ConnectionRow {
  id: string;
  kind: ConnectionKind;
  name: string;
  settings: Record<string, unknown>;
  has_secret: boolean;
  last_test: { at: number; ok: boolean; note: string } | null;
}

export const mcpList = () => invoke<ServerRow[]>("mcp_list");
export const mcpServer = (name: string) => invoke<ServerDetail>("mcp_server", { name });
export const mcpCatalog = () => invoke<{ version: number; checked: string; entries: CatalogEntry[] }>("mcp_catalog");
export const mcpAdd = (request: AddServer) => invoke<ServerRow>("mcp_add", { request });
export const mcpSetEnabled = (name: string, enabled: boolean) => invoke<ServerRow>("mcp_set_enabled", { name, enabled });
export const mcpRemove = (name: string) => invoke<void>("mcp_remove", { name });
export const mcpRestart = (name: string) => invoke<ServerRow>("mcp_restart", { name });
export const mcpRefresh = (name: string) => invoke<void>("mcp_refresh", { name });
export const mcpReview = (name: string, tool: string, action: "approve" | "disable") => invoke<void>("mcp_review", { name, tool, action });
export const mcpApproveAll = (name: string) => invoke<number>("mcp_approve_all", { name });
export const mcpTrust = (name: string, tool: string, skipAsking: boolean) => invoke<void>("mcp_trust", { name, tool, skipAsking });
export const mcpLogin = (name: string) => invoke<string>("mcp_login", { name });
export const mcpLoginCancel = (name: string) => invoke<void>("mcp_login_cancel", { name });
export const mcpLogout = (name: string) => invoke<void>("mcp_logout", { name });
export const mcpImportPreview = (source: string, path?: string) => invoke<ImportPreview[]>("mcp_import_preview", { source, path });
export const mcpImportApply = (source: string, path: string, names: string[]) => invoke<string[]>("mcp_import_apply", { source, path, names });
export const connectionsList = () => invoke<ConnectionRow[]>("connections_list");
export const connectionAdd = (request: { kind: ConnectionKind; name: string; settings: Record<string, unknown>; secret?: string }) =>
  invoke<ConnectionRow>("connection_add", { request });
export const connectionTest = (name: string) => invoke<{ ok: boolean; note: string }>("connection_test", { name });
export const connectionRemove = (name: string) => invoke<void>("connection_remove", { name });
export const sandboxStatus = () => invoke<{ javascript: boolean; python: boolean; python_megabytes: number }>("sandbox_status");
export function sandboxInstallPython(onProgress: (done: number, total: number) => void): Promise<void> {
  const channel = new Channel<[number, number]>();
  channel.onmessage = ([done, total]) => onProgress(done, total);
  return invoke<void>("sandbox_install_python", { progress: channel });
}

/** Be told when any MCP server changes state (the value is the server's id, or `*` when some were missed). */
export function onMcpStatus(handler: () => void): Promise<UnlistenFn> {
  return listen<string>("mcp-status", () => handler());
}
