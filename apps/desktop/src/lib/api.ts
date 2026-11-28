// The only file that talks to the Rust side. Types mirror what the engine sends (serde JSON).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { CheckedFile } from "./attach.ts";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import { t as tr } from "./i18n/index.ts";

export type ExecutionState =
  | "created"
  | "queued"
  | "running"
  | "waiting_for_model"
  | "waiting_for_tool"
  | "waiting_for_approval"
  | "waiting_for_children"
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
  request: { prompt: string; instructions: string | null; model: string; metadata?: Record<string, string> };
  state: ExecutionState;
  version: number;
  iteration: number;
  tool_calls_requested: number;
  active_ms: number;
  usage: Usage;
  conversation?: string | null;
  /** The agent (and version) that made this run, when one did. */
  agent?: { agent: string; slug: string; version: number; hash: string; team?: string | null } | null;
  /** The run that asked for this one, when work was delegated. */
  parent?: string | null;
  /** The run at the top of its tree (none for a root). */
  root?: string | null;
  /** How deep it is: 0 for a root. */
  depth?: number;
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
  status: "requested" | "awaiting_approval" | "started" | "succeeded" | "failed" | "denied" | "delegated";
  attempts: number;
  error: string | null;
  /** What the tool answered once it succeeded. For a delegation: the sub-runs started, then their results. */
  result?: unknown;
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
  /** The most the run may spend, in millionths of a dollar. Left out: no limit. */
  max_cost_micros?: number;
  /** The most seconds of work. Left out: the default. */
  max_seconds?: number;
  /** `provider/model`, `route:name`, an alias or `demo`. Left out: the default. */
  model?: string;
  /** Continue this conversation. Left out: a new one starts. */
  conversation?: string;
  /** What the run may use from memory. Left out: everything. */
  memory?: { conversation: boolean; long_term: boolean; knowledge: boolean; operations: boolean };
  /** Files the run is given to read, by path: at most ten, each checked before the run exists. */
  attachments?: string[];
  /** The servers and services the run may reach. Each entry means what the matching `--allow-…` flag means. */
  integrations?: Partial<Record<"mcp" | "http" | "repo" | "jira" | "chat" | "db" | "code" | "git" | "git_push", string[]>>;
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
  /** Files whose text came from reading the pictures of their pages. */
  ocr_read: number;
  /** Scanned PDFs that are left out: reading scans is off, or the reading failed. */
  scans_unread: number;
  syncing: boolean;
  /** Set to be kept up to date while the app runs. */
  watch: boolean;
  /** How it is kept up to date now: the system says when something changes, or it is checked every ten minutes, or neither. */
  watching: "watching" | "polling" | "off";
  /** Why it is checked on a timer and not watched. */
  watch_note: string | null;
}

export interface NewRoot {
  path: string;
  project?: string;
  include?: string[];
  exclude?: string[];
  auto_sync?: boolean;
  /** Keep the index up to date while the app runs. Left out: on. */
  watch?: boolean;
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

/** Where reading scanned PDFs stands: the setting, and whether the models are loaded or being downloaded. */
export interface OcrStatus {
  enabled: boolean;
  state: "off" | "ready" | "missing" | "installing";
  /** Bytes received of the download, while there is one. */
  done: number;
  /** Bytes the download will have. */
  total: number;
}

/** Check files as they are chosen or dropped: nothing is read, and a refusal comes back as a sentence. */
export const attachmentsCheck = (paths: string[], workspace?: string) =>
  invoke<CheckedFile[]>("attachments_check", { paths, workspace: workspace ?? null });

/** One file the `@` menu offers. */
export interface AttachCandidate {
  path: string;
  name: string;
  folder: string;
}

/** What the `@` menu lists for what was typed: indexed documents, and files of the workspace folder. */
export interface AttachCandidates {
  documents: AttachCandidate[];
  files: AttachCandidate[];
}

export const attachmentCandidates = (query: string, workspace?: string) =>
  invoke<AttachCandidates>("attachment_candidates", { query, workspace: workspace ?? null });

export const ocrStatus = () => invoke<OcrStatus>("ocr_status");
/** Turn reading scans on (downloading the models the first time) or off. Resolves when it is done. */
export const ocrSet = (enabled: boolean) => invoke<OcrStatus>("ocr_set", { enabled });
/** Follow the download of the models. Returns a function that stops listening. */
export async function onOcrProgress(handler: (p: { done: number; total: number }) => void): Promise<() => void> {
  const stop = await listen<{ done: number; total: number }>("ocr://progress", (e) => handler(e.payload));
  return () => stop();
}

export const listRoots = () => invoke<RootRow[]>("list_roots");
export const addRoot = (root: NewRoot) => invoke<RootRow>("add_root", { root });
export const setRootWatch = (id: string, on: boolean) => invoke<RootRow>("set_root_watch", { id, on });
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

/** Tell the shell the interface has loaded (it only does something when a check asked for it with ZORIX_SMOKE). */
export const appReady = () => invoke<void>("app_ready");
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

export { type UiError, errorText } from "./errors.ts";

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
  /** The id of the catalogue entry it came from, when there is one. */
  catalog: string | null;
  logo: string | null;
}

export interface McpToolRow {
  name: string;
  /** The hash of this definition: approving sends it back, so what is approved is what was shown. */
  hash: string;
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
  /** The sandbox of a local server; null for a remote one, and for one added before sandboxes. */
  sandbox: ServerSandbox | null;
  enforced: SandboxEnforced;
}

export interface SandboxFolder {
  path: string;
  write: boolean;
}

export interface ServerSandbox {
  mode: "native" | "container" | "off";
  network: boolean;
  folders: SandboxFolder[];
}

export interface SandboxEnforced {
  os: string;
  /** The container program found here, if any. */
  container: string | null;
  files: boolean;
  network: boolean;
  credentials: boolean;
  processes: boolean;
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
export const mcpReview = (name: string, tool: string, action: "approve" | "disable", hash: string | null = null) => invoke<void>("mcp_review", { name, tool, action, hash });
export const mcpApproveAll = (name: string, shown: [string, string][]) => invoke<number>("mcp_approve_all", { name, shown });
export const mcpTrust = (name: string, tool: string, skipAsking: boolean) => invoke<void>("mcp_trust", { name, tool, skipAsking });
export const mcpSetSandbox = (name: string, sandbox: ServerSandbox) => invoke<void>("mcp_set_sandbox", { name, sandbox });
export const mcpLogin = (name: string) => invoke<string>("mcp_login", { name });
export const mcpLoginCancel = (name: string) => invoke<void>("mcp_login_cancel", { name });
export const mcpLogout = (name: string) => invoke<void>("mcp_logout", { name });
export const mcpImportPreview = (source: string, path?: string) => invoke<ImportPreview[]>("mcp_import_preview", { source, path });
export const mcpImportApply = (source: string, path: string, names: string[]) => invoke<string[]>("mcp_import_apply", { source, path, names });
export const connectionsList = () => invoke<ConnectionRow[]>("connections_list");
export const connectionAdd = (request: { kind: ConnectionKind; name: string; settings: Record<string, unknown>; secret?: string }) =>
  invoke<ConnectionRow>("connection_add", { request });
export interface SignInInfo {
  builtin: boolean;
  scopes: string;
  redirect: string | null;
  needs_secret: boolean;
}
export interface DeviceStep {
  code: string;
  url: string;
}
export const connectionSignInInfo = (kind: ConnectionKind) => invoke<SignInInfo | null>("connection_sign_in_info", { kind });
export function connectionSignIn(
  request: { kind: ConnectionKind; name: string; settings: Record<string, unknown> },
  app: { clientId?: string; clientSecret?: string },
  onStep: (step: DeviceStep) => void,
): Promise<ConnectionRow> {
  const steps = new Channel<DeviceStep>();
  steps.onmessage = onStep;
  return invoke<ConnectionRow>("connection_sign_in", { request, clientId: app.clientId ?? null, clientSecret: app.clientSecret ?? null, steps });
}
export const connectionSignInOpen = (name: string) => invoke<void>("connection_sign_in_open", { name });
export const connectionSignInCancel = (name: string) => invoke<void>("connection_sign_in_cancel", { name });
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


// ---------------------------------------------------------------------------------------------------------------------
// What an MCP server asks of you while a tool runs (sampling and elicitation)

export type AskFieldKind = { type: "text" | "number" | "integer" | "boolean" } | { type: "choice"; options: string[] };
export interface AskField {
  name: string;
  title?: string;
  description?: string;
  kind: AskFieldKind;
  required: boolean;
}
export type ServerRequest =
  | { kind: "model"; system?: string; messages: { role: "user" | "assistant"; text: string }[]; max_tokens: number }
  | { kind: "person"; mode: "form"; message: string; fields: AskField[] }
  | { kind: "person"; mode: "link"; message: string; url: string };
export interface PendingAsk {
  id: string;
  execution_id: string;
  call_id: string;
  server: string;
  tool: string;
  model: string;
  ask: ServerRequest;
  hash: string;
  asked_at: number;
}
export type AskAnswer = { answer: "yes"; values: Record<string, unknown> } | { answer: "no" };

/** What an approval of `git.commit` or `git.push` is about: the diff and, for a push, where it goes. */
export interface GitPreview {
  diff: string;
  branch: string;
  files: string[];
  new_files?: string[];
  not_shown?: number;
  url?: string;
  new_on_remote?: boolean;
  commit?: string;
  subject?: string;
}
export const gitPreview = (executionId: string, approvalId: string) => invoke<GitPreview>("git_preview", { executionId, approvalId });

export const serverAsks = () => invoke<PendingAsk[]>("server_asks");
export const answerServerAsk = (id: string, hash: string, answer: AskAnswer) => invoke<void>("server_ask_answer", { id, hash, answer });
/** Be told when the list of waiting requests changes. */
export function onServerAsks(handler: () => void): Promise<UnlistenFn> {
  return listen<null>("server-asks", () => handler());
}

// ---------------------------------------------------------------------------------------------------------------------
// Agents, skills and evaluations

export interface AgentCard {
  id: string;
  slug: string;
  name: string;
  description: string;
  emoji: string;
  color: string;
  default_model: string;
  version: number;
  skills: string[];
  archived: boolean;
}

export interface Limits {
  max_iterations: number;
  max_tool_calls: number;
  max_total_tokens: number | null;
  max_cost_micros: number | null;
  max_active_ms: number;
  model_timeout_ms: number;
  max_model_attempts: number;
  max_validation_retries: number;
}

export interface AgentSpec {
  slug: string;
  name: string;
  description: string;
  avatar: { emoji: string; color: string };
  instructions: string;
  skills: { name: string; hash: string }[];
  tools: string[];
  access: string[];
  models: { default: string; allowed: string[] };
  limits: Limits;
  memory: { conversation: boolean; long_term: boolean; knowledge: "off" | "auto"; operations: "off" | "auto" };
  agent_memory: boolean;
  params: { max_output_tokens: number | null; effort: string | null };
  expected_output: unknown;
  metadata: Record<string, string>;
}

export interface AgentVersion {
  agent: string;
  version: number;
  hash: string;
  spec: AgentSpec;
  note: string | null;
  created_at: number;
}

export interface SkillFile {
  path: string;
  size: number;
  sha256: string;
}

export interface SkillRecord {
  name: string;
  hash: string;
  description: string;
  files: SkillFile[];
  license: string | null;
  compatibility: string | null;
  metadata: Record<string, string>;
  allowed_tools: string[];
  source: string | null;
  imported_at: number;
}

export interface AgentDetail {
  agent: { id: string; slug: string; current_version: number; archived: boolean; created_at: number; updated_at: number };
  version: AgentVersion;
  skills: SkillRecord[];
  cases: number;
}

export interface Saved {
  slug: string;
  version: number;
  created: boolean;
  changed: boolean;
}

export interface Template {
  id: string;
  name: string;
  description: string;
  emoji: string;
  color: string;
  skills: string[];
  cases: number;
  installed: boolean;
}

export interface DiffRow {
  change: "same" | "added" | "removed";
  text: string;
}

export type EvalCheck =
  | { kind: "contains" | "not_contains"; text: string; ignore_case?: boolean }
  | { kind: "regex"; pattern: string }
  | { kind: "json_schema"; schema: unknown }
  | { kind: "tool_used"; pattern: string; min?: number }
  | { kind: "tool_not_used"; pattern: string }
  | { kind: "max_cost_usd"; usd: number }
  | { kind: "max_steps"; steps: number }
  | { kind: "max_tool_calls"; calls: number }
  | { kind: "status"; state: string };

export interface EvalCase {
  name: string;
  prompt: string;
  input?: unknown;
  tags: string[];
  checks: EvalCheck[];
  judge?: { rubric: string; model: string; pass_score: number } | null;
  approve: string[];
  allow_writes: boolean;
}

export interface StoredCase {
  id: string;
  agent: string;
  case: EvalCase;
  updated_at: number;
}

export interface EvalResult {
  case: string;
  execution: string | null;
  passed: boolean;
  checks: { check: string; passed: boolean; detail: string }[];
  judge: { score: number; reasons: string; cost_micros: number; model: string } | null;
  state: string;
  steps: number;
  tool_calls: number;
  cost_micros: number;
  error: string | null;
}

export interface EvalRun {
  id: string;
  agent: string;
  version: number;
  model: string;
  status: "running" | "finished" | "cancelled";
  started_at: number;
  finished_at: number | null;
  results: EvalResult[];
}

export interface CompareRow {
  case: string;
  change: "StillPasses" | "StillFails" | "Fixed" | "Broke" | "OnlyInOne";
  cost_delta_micros: number;
  steps_delta: number;
  score_delta: number | null;
}

export interface AgentConversation {
  conversation: string;
  title: string;
  runs: number;
  last_at: number;
}

export interface ThreadRun {
  execution: Execution;
  tool_calls: ToolCall[];
  approvals: Approval[];
}

export interface AskAgent {
  slug: string;
  version?: number;
  prompt: string;
  input?: unknown;
  model?: string;
  conversation?: string;
  /** Ask a team: the agent named above is its lead. */
  team?: string;
  /** What it may use from memory: each switch is on only if the agent allows it too. */
  memory?: { conversation: boolean; long_term: boolean; knowledge: boolean; operations: boolean };
  /** Limits that can only narrow the agent's own. */
  max_cost_micros?: number;
  max_seconds?: number;
  max_iterations?: number;
  /** Files the run is given to read, by path. */
  attachments?: string[];
}

export const agentsList = (includeArchived = false) => invoke<AgentCard[]>("agents_list", { includeArchived });
export const agentGet = (slug: string, version?: number) => invoke<AgentDetail>("agent_get", { slug, version });
export const agentSave = (spec: AgentSpec, note?: string) => invoke<Saved>("agent_save", { spec, note });
export const agentImport = (file: string, rename?: string) => invoke<Saved>("agent_import", { file, rename });
export const agentExport = (slug: string, version?: number) => invoke<string>("agent_export", { slug, version });
export const agentVersions = (slug: string) => invoke<AgentVersion[]>("agent_versions", { slug });
export const agentDiff = (slug: string, from: number, to: number) => invoke<DiffRow[]>("agent_diff", { slug, from, to });
export const agentRollback = (slug: string, to: number) => invoke<AgentVersion>("agent_rollback", { slug, to });
export const agentArchive = (slug: string, archived: boolean) => invoke<void>("agent_archive", { slug, archived });
export const agentRemove = (slug: string) => invoke<void>("agent_remove", { slug });
export const agentTemplates = () => invoke<Template[]>("agent_templates");
export const agentAddTemplate = (id: string, rename?: string) => invoke<Saved>("agent_add_template", { id, rename });
export const skillsList = () => invoke<SkillRecord[]>("skills_list");
export const skillImport = (path: string) => invoke<SkillRecord>("skill_import", { path });
export const skillRemove = (name: string) => invoke<void>("skill_remove", { name });
export const skillFile = (hash: string, path: string) => invoke<string>("skill_file", { hash, path });
export const evalCases = (slug: string) => invoke<StoredCase[]>("eval_cases", { slug });
export const evalPutCase = (slug: string, evalCase: EvalCase) => invoke<StoredCase>("eval_put_case", { slug, case: evalCase });
export const evalRemoveCase = (slug: string, name: string) => invoke<void>("eval_remove_case", { slug, name });
export const evalRuns = (slug: string) => invoke<EvalRun[]>("eval_runs", { slug });
export const evalRunning = (slug: string) => invoke<boolean>("eval_running", { slug });
export const evalStart = (request: { slug: string; version?: number; models?: string[]; tags?: string[]; offline?: boolean }) =>
  invoke<void>("eval_start", { request });
export const evalCancel = (slug: string) => invoke<void>("eval_cancel", { slug });
export const evalCompare = (before: string, after: string) => invoke<CompareRow[]>("eval_compare", { before, after });
export const agentAsk = (request: AskAgent) => invoke<Execution>("agent_ask", { request });
export const agentThreads = (slug: string) => invoke<AgentConversation[]>("agent_threads", { slug });
export const threadRuns = (slug: string, conversation: string) => invoke<ThreadRun[]>("thread_runs", { slug, conversation });
export const stopLive = (id: string) => invoke<void>("stop_live", { id });
export const openExternal = (url: string) => invoke<void>("open_external", { url });

export interface LiveBatch {
  execution: string;
  text: string;
  /** Forget what was drawn before: another model took over. */
  reset: boolean;
}

/** The text of a run's answer while it is still being written. Resolves when the watch ends. */
export function watchLive(id: string, onBatch: (batch: LiveBatch) => void): Promise<void> {
  const channel = new Channel<LiveBatch>();
  channel.onmessage = onBatch;
  return invoke<void>("watch_live", { id, onText: channel });
}

// ---------------------------------------------------------------------------------------------------------------------
// Trees of work and teams

export interface TreeApproval {
  /** The execution that waits (a sub-run, or the root itself). */
  execution: string;
  agent: string | null;
  depth: number;
  approval: Approval;
}

export interface Team {
  tenant: string;
  slug: string;
  name: string;
  description: string;
  avatar: { emoji: string; color: string };
  lead: string;
  members: string[];
  updated_at: number;
}

export interface TeamView {
  team: Team;
  lead: AgentCard | null;
  members: AgentCard[];
  /** Names the team lists that are not agents any more. */
  missing: string[];
}

export interface TeamTemplate {
  id: string;
  name: string;
  description: string;
  emoji: string;
  color: string;
  lead: string;
  members: string[];
  installed: boolean;
}

export interface SaveTeam {
  slug: string;
  name: string;
  description?: string;
  emoji?: string;
  color?: string;
  lead: string;
  members: string[];
}

export const runTree = (id: string) => invoke<Execution[]>("run_tree", { id });
export const treeApprovals = (id: string) => invoke<TreeApproval[]>("tree_approvals", { id });
export const subRunCounts = (ids: string[]) => invoke<Record<string, number>>("sub_run_counts", { ids });
export const teamsList = () => invoke<TeamView[]>("teams_list");
export const teamSave = (team: SaveTeam) => invoke<TeamView>("team_save", { team });
export const teamRemove = (slug: string) => invoke<void>("team_remove", { slug });
export const teamTemplates = () => invoke<TeamTemplate[]>("team_templates");
export const teamAddTemplate = (id: string) => invoke<string>("team_add_template", { id });

// ---- Workflows ------------------------------------------------------------------------------------------------------------

export type Day = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";
export type When = { type: "every"; minutes: number } | { type: "daily"; times: string[]; days: Day[] } | { type: "cron"; expr: string };
export type EventOn = "finished" | "failed" | "any";
export type Then = "refuse" | "continue";
export type Ask = "always" | "never";
export interface Remind {
  after_ms: number;
  connection: string;
  channel: string;
}
export type StepKind =
  | { type: "manual" }
  | { type: "schedule"; when: When; timezone?: string | null }
  | { type: "webhook"; source: string; label?: string | null; filter?: string | null; signed?: boolean }
  | { type: "event"; on: EventOn; agent?: string | null }
  | { type: "after_workflow"; workflow: string; on: EventOn }
  | { type: "agent"; agent: string; prompt: string; version?: number | null; model?: string | null; max_cost_micros?: number | null }
  | { type: "team"; team: string; prompt: string; model?: string | null; max_cost_micros?: number | null }
  | { type: "condition"; expr: string }
  | { type: "loop"; over: string; agent: string; prompt: string }
  | { type: "wait"; for_ms?: number | null; until?: string | null }
  | { type: "approval"; message: string; show?: string[]; timeout_ms?: number | null; then?: Then; remind?: Remind | null }
  | { type: "question"; question: string; default?: string | null; timeout_ms?: number | null; then?: Then }
  | { type: "send"; service: "slack" | "discord"; connection?: string | null; channel: string; text: string; ask?: Ask }
  | { type: "http"; method: string; url: string; body?: string | null; ask?: Ask };
export type StepKindName = StepKind["type"];
export interface WfStep {
  id: string;
  title: string;
  x: number;
  y: number;
  kind: StepKind;
}
export interface WfEdge {
  from: string;
  to: string;
  label?: string | null;
}
export interface WorkflowSpec {
  name: string;
  description?: string;
  group?: string;
  steps: WfStep[];
  edges: WfEdge[];
  budget_micros?: number | null;
  saves_minutes?: number | null;
}
export interface Workflow {
  id: string;
  tenant: string;
  slug: string;
  draft: WorkflowSpec;
  live_version: number | null;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}
export type RunStateName = "running" | "waiting" | "completed" | "failed" | "cancelled";
export type StepStateName = "pending" | "running" | "waiting" | "done" | "failed" | "skipped";
export interface WfStepRun {
  state: StepStateName;
  started_at?: number | null;
  finished_at?: number | null;
  output?: unknown;
  error?: string | null;
  note?: string | null;
  execution?: string | null;
  executions?: string[];
  wait?: string | null;
  cost_micros: number;
}
export interface WfRun {
  id: string;
  slug: string;
  workflow: string;
  version: number | null;
  test: boolean;
  spec: WorkflowSpec;
  fired: { step: string; label: string; payload: unknown; at: number };
  state: RunStateName;
  steps: Record<string, WfStepRun>;
  cost_micros: number;
  error?: string | null;
  chain: number;
  from_run?: string | null;
  started_at: number;
  finished_at?: number | null;
  revision: number;
}
export type WaitKindName = "approval" | "question" | "action" | "timer";
export interface WfWait {
  id: string;
  run: string;
  workflow: string;
  step: string;
  kind: WaitKindName;
  title: string;
  message: string;
  shown: Record<string, unknown>;
  fingerprint: string;
  status: "pending" | "approved" | "refused" | "answered" | "elapsed" | "timed_out" | "cancelled";
  due_at?: number | null;
  then: Then;
  default_answer?: string | null;
  created_at: number;
  decided_at?: number | null;
  decided_by?: string | null;
  answer?: string | null;
  note?: string | null;
}
export type Health = { kind: "paused" } | { kind: "waiting"; count: number } | { kind: "failed"; count: number } | { kind: "healthy" };
export interface WfBar {
  run: string;
  state: RunStateName;
  waited: boolean;
  /** How long the run took, in milliseconds: the height of its bar. */
  ms: number;
}
export interface WfCard {
  workflow: Workflow;
  triggers: { step: string; kind: string; label: string }[];
  bars: WfBar[];
  health: Health;
  unpublished_changes: boolean;
  missed: number;
}
export interface WfFigures {
  active: number;
  paused: number;
  runs_week: number;
  finished_percent: number | null;
  waiting: number;
  oldest_wait_ms: number | null;
  saved_minutes: number | null;
}
export interface HostInfo {
  running: boolean;
  webhooks: boolean;
  listening: string | null;
  address: string;
}
export interface WfOverview {
  cards: WfCard[];
  figures: WfFigures;
  host: HostInfo;
}
export interface WfVersionRow {
  version: number;
  hash: string;
  note: string | null;
  created_at: number;
  confirmed: number;
}
export interface WfMissing {
  step: string;
  kind: "agent" | "team" | "connection";
  name: string;
}
export interface WfDetail {
  workflow: Workflow;
  versions: WfVersionRow[];
  missing: WfMissing[];
  to_confirm: string[];
  unpublished_changes: boolean;
}
export interface WfTemplate {
  id: string;
  name: string;
  description: string;
  group: string;
  triggers: string[];
  steps: number;
}
export interface WfDiffLine {
  kind: "same" | "added" | "removed";
  text: string;
}
export interface WfRunDetail {
  run: WfRun;
  waits: WfWait[];
}
export interface WfStepHistory {
  outcomes: StepStateName[];
  median_wait_ms: number | null;
}
export interface WfWebhook {
  url: string;
  signing_secret: string | null;
  listening: boolean;
}

/** The settings of the program that are kept in the database (`AppSettings` in `zorix-core`). */
export interface AppSettings {
  context: { summaries: boolean };
  /** Finished runs older than `days` are compacted or deleted; `null` keeps everything for ever. */
  retention: { days: number | null; mode: "compact" | "delete" };
  /** An OpenTelemetry collector (OTLP over HTTP); `null` sends nothing. */
  /** `header` is only the name of a header the collector needs: its value is kept in the system keychain and is never returned. */
  telemetry: { endpoint: string | null; header?: string | null };
  knowledge: { ocr: boolean };
}

/** What a pass of retention did, or would do. */
export interface RetentionReport {
  runs: number;
  compacted: number;
  deleted: number;
  kept_because_referenced: number;
  events: number;
  checkpoints: number;
  tool_calls: number;
  approvals: number;
  snapshots: number;
  bytes: number;
}

export const appSettings = () => invoke<AppSettings>("app_settings_get");
export const telemetrySetHeader = (name: string | null, value: string | null) =>
  invoke<AppSettings>("telemetry_set_header", { name, value });
export const retentionPreview = (days: number, mode: "compact" | "delete") => invoke<RetentionReport>("retention_preview", { days, mode });
export const retentionRun = () => invoke<RetentionReport>("retention_run");

/** What a run would probably cost and how long it would take. */
export interface RunEstimate {
  basis: { kind: "history"; runs: number } | { kind: "rough" };
  input_tokens: number;
  output_tokens: number;
  active_ms: number;
  /** Millionths of a dollar; `null` when the model has no known price. */
  cost_micros: number | null;
}

export const estimateRun = (promptChars: number, attachedChars: number, model?: string, agent?: string) =>
  invoke<RunEstimate>("estimate_run", { promptChars, attachedChars, model: model || null, agent: agent || null });
export const appSettingsSet = (settings: AppSettings) => invoke<AppSettings>("app_settings_set", { settings });

export const workflowHost = () => invoke<HostInfo>("workflow_host");
export const workflowSettingsSet = (webhooks: boolean, address: string) => invoke<HostInfo>("workflow_settings_set", { webhooks, address });
export const workflowsOverview = () => invoke<WfOverview>("workflows_overview");
export const workflowDetail = (id: string) => invoke<WfDetail>("workflow_detail", { id });
export const workflowCreate = (slug: string, name: string) => invoke<Workflow>("workflow_create", { slug, name });
export const workflowSave = (id: string, spec: WorkflowSpec) => invoke<Workflow>("workflow_save", { id, spec });
export const workflowPublish = (id: string, note: string | null, confirm: boolean) =>
  invoke<{ version: number; written: boolean }>("workflow_publish", { id, note, confirm });
export const workflowRollback = (id: string, to: number, confirm: boolean) =>
  invoke<{ version: number; written: boolean }>("workflow_rollback", { id, to, confirm });
export const workflowSetEnabled = (id: string, enabled: boolean) => invoke<void>("workflow_set_enabled", { id, enabled });
export const workflowRemove = (id: string) => invoke<boolean>("workflow_remove", { id });
export const workflowExport = (id: string, version?: number) => invoke<string>("workflow_export", { id, version: version ?? null });
export const workflowImport = (contents: string) => invoke<{ workflow: Workflow; created: boolean; changed: boolean }>("workflow_import", { contents });
export const workflowTemplates = () => invoke<WfTemplate[]>("workflow_templates");
export const workflowAddTemplate = (template: string) => invoke<{ workflow: Workflow; missing: WfMissing[] }>("workflow_add_template", { template });
export const workflowDiff = (id: string, from: string, to: string) => invoke<WfDiffLine[]>("workflow_diff", { id, from, to });
export const workflowOutline = (id: string, which: string) => invoke<string>("workflow_outline", { id, which });
export const workflowStart = (id: string, test: boolean, payload?: unknown, trigger?: string) =>
  invoke<WfRun>("workflow_start", { request: { id, test, payload: payload ?? {}, trigger: trigger ?? null } });
export const workflowRuns = (id?: string, limit?: number) => invoke<WfRun[]>("workflow_runs", { id: id ?? null, limit: limit ?? null });
export const workflowRun = (run: string) => invoke<WfRunDetail>("workflow_run", { run });
export const workflowCancel = (run: string) => invoke<WfRun>("workflow_cancel", { run });
export const workflowWaits = () => invoke<WfWait[]>("workflow_waits");
export const workflowDecide = (wait: string, fingerprint: string, verdict: "approve" | "refuse" | "answer", text?: string, note?: string) =>
  invoke<WfWait>("workflow_decide", { request: { wait, fingerprint, verdict, text: text ?? null, note: note ?? null } });
export const workflowStepHistory = (id: string, step: string) => invoke<WfStepHistory>("workflow_step_history", { id, step });
export const workflowWebhook = (id: string, reset = false) => invoke<WfWebhook>("workflow_webhook", { id, reset });

// ---------------------------------------------------------------------------------------------------------------------
// Projects and tasks

export type Who = { type: "you" } | { type: "agent"; slug: string } | { type: "team"; slug: string };
export type TaskState = "backlog" | "in_progress" | "review" | "done";
export type TaskPriority = "urgent" | "high" | "normal" | "low";
export type RunOutcome = "running" | "waiting" | "finished" | "failed" | "cancelled";
export type NeedsYou = "review" | "run_failed" | "run_waiting";

export interface TaskRun {
  execution: string;
  started_at: number;
  ended_at: number | null;
  outcome: RunOutcome;
  note: string | null;
  reason: string | null;
  cost_micros: number;
}

export interface TaskLink {
  url: string;
  title: string | null;
}

export interface Deliverable {
  id: string;
  kind: "answer" | "link" | "file";
  title: string;
  body: string;
  by: Who | null;
  at: number;
  execution: string | null;
}

export interface Task {
  id: string;
  project: string;
  key: string;
  title: string;
  description: string;
  state: TaskState;
  priority: TaskPriority;
  assignee: Who | null;
  progress: number;
  start: string | null;
  due: string | null;
  depends_on: string[];
  links: TaskLink[];
  deliverables: Deliverable[];
  runs: TaskRun[];
  run_requested: { note: string | null; error: string | null } | null;
  rank: number;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface TaskView {
  task: Task;
  blocked_by: string[];
  needs_you: NeedsYou | null;
  working: boolean;
}

export interface Project {
  id: string;
  key: string;
  name: string;
  description: string;
  manager: string | null;
  start: string | null;
  due: string | null;
  cost_limit_micros: number | null;
  archived: boolean;
  next_task: number;
  created_at: number;
  updated_at: number;
}

export interface Counts {
  backlog: number;
  in_progress: number;
  review: number;
  done: number;
}

export interface ProjectCard {
  project: Project;
  counts: Counts;
  needs_you: number;
  assignees: Who[];
}

export interface Board {
  project: Project;
  tasks: TaskView[];
  manager_run: { execution: string; waiting: boolean } | null;
}

export type ProjectEventKind =
  | { type: "project_created" }
  | { type: "project_edited" }
  | { type: "project_archived"; archived: boolean }
  | { type: "task_created"; title: string }
  | { type: "task_edited" }
  | { type: "moved"; from: TaskState; to: TaskState; approved_by?: string; note?: string }
  | { type: "assigned"; to?: Who }
  | { type: "run_started"; execution: string }
  | { type: "run_finished"; execution: string }
  | { type: "run_failed"; execution: string; reason: string }
  | { type: "start_failed"; reason: string }
  | { type: "run_cancelled"; execution: string }
  | { type: "blocked"; by: string[] }
  | { type: "progress"; percent: number }
  | { type: "deliverable"; title: string }
  | { type: "comment"; text: string }
  | { type: "linked"; url: string }
  | { type: "manager_asked"; execution: string };

export interface ProjectEvent {
  seq: number;
  project: string;
  task: string | null;
  by: Who | null;
  kind: ProjectEventKind;
  at: number;
}

export interface PullState {
  number: number;
  title: string;
  state: string;
  draft: boolean;
}

export interface TaskPage {
  task: TaskView;
  events: ProjectEvent[];
  links: { url: string; pull: PullState | null }[];
}

export interface TaskNeed {
  key: string;
  title: string;
  project: string;
  project_name: string;
  needs: NeedsYou;
  assignee: Who | null;
  at: number;
}

export interface TaskIndexRow {
  key: string;
  title: string;
  project: string;
  state: TaskState;
}

export interface NewProjectRequest {
  key: string;
  name: string;
  description: string;
  manager: string | null;
  start: string | null;
  due: string | null;
  cost_limit_micros: number | null;
}

export interface ProjectFields {
  name: string;
  description: string;
  manager: string | null;
  start: string | null;
  due: string | null;
  cost_limit_micros: number | null;
  archived: boolean;
}

export interface NewTaskRequest {
  title: string;
  description?: string;
  state?: TaskState | null;
  priority?: TaskPriority;
  assignee?: Who | null;
  start?: string | null;
  due?: string | null;
  depends_on?: string[];
  links?: TaskLink[];
}

export interface TaskFields {
  title: string;
  description: string;
  priority: TaskPriority;
  assignee: Who | null;
  start: string | null;
  due: string | null;
  depends_on: string[];
  links: TaskLink[];
  progress: number;
}

export interface TaskMove {
  to: TaskState;
  index?: number | null;
  note?: string | null;
  cancel_run?: boolean;
}

/** The words a failed move starts with when a run is going and would be stopped. */
export const RUN_GOING = tr("api.runGoing");

// The store leaves out lists that are empty and fields that are not set; the screens always get all of them.
const fillTask = (t: Partial<Task> & Pick<Task, "key">): Task =>
  ({
    description: "",
    assignee: null,
    start: null,
    due: null,
    progress: 0,
    depends_on: [],
    links: [],
    deliverables: [],
    runs: [],
    run_requested: null,
    ...t,
  }) as Task;
const fillView = (v: TaskView): TaskView => ({ ...v, task: fillTask(v.task) });
const fillProject = (p: Project): Project => ({
  ...p,
  description: p.description ?? "",
  manager: p.manager ?? null,
  start: p.start ?? null,
  due: p.due ?? null,
  cost_limit_micros: p.cost_limit_micros ?? null,
});

export const projectsOverview = (archived = false) =>
  invoke<ProjectCard[]>("projects_overview", { archived }).then((cards) => cards.map((c) => ({ ...c, project: fillProject(c.project) })));
export const projectCreate = (request: NewProjectRequest) => invoke<Project>("project_create", { new: request }).then(fillProject);
export const projectUpdate = (key: string, fields: ProjectFields) => invoke<Project>("project_update", { key, fields }).then(fillProject);
export const projectRemove = (key: string) => invoke<void>("project_remove", { key });
export const projectBoard = (key: string) =>
  invoke<Board>("project_board", { key }).then((b) => ({ ...b, project: fillProject(b.project), tasks: b.tasks.map(fillView) }));
export const projectAsk = (key: string, prompt?: string) => invoke<string>("project_ask", { key, prompt: prompt ?? null });
export const projectHistory = (key: string, limit = 100) => invoke<ProjectEvent[]>("project_history", { key, limit });
export const taskAdd = (project: string, request: NewTaskRequest) => invoke<Task>("task_add", { project, new: request }).then(fillTask);
export const taskUpdate = (key: string, fields: TaskFields) => invoke<Task>("task_update", { key, fields }).then(fillTask);
export const taskMove = (key: string, mv: TaskMove) => invoke<Task>("task_move", { key, mv }).then(fillTask);
export const taskRestart = (key: string, note?: string) => invoke<Task>("task_restart", { key, note: note ?? null }).then(fillTask);
export const taskRemove = (key: string) => invoke<void>("task_remove", { key });
export const taskDetail = (key: string) => invoke<TaskPage>("task_detail", { key }).then((p) => ({ ...p, task: fillView(p.task) }));
export const taskComment = (key: string, body: string) => invoke<void>("task_comment", { key, body });
export const taskLink = (key: string, url: string, title?: string) => invoke<Task>("task_link", { key, url, title: title ?? null }).then(fillTask);
export const taskDeliver = (key: string, kind: "answer" | "link", title: string, body: string) =>
  invoke<Task>("task_deliver", { key, kind, title, body }).then(fillTask);
export const tasksNeedingYou = () => invoke<TaskNeed[]>("tasks_needing_you");
export const tasksIndex = () => invoke<TaskIndexRow[]>("tasks_index");

// ---------------------------------------------------------------------------------------------------------------------
// The workforce

export type AutonomyLevel = "suggest" | "ask" | "notify" | "budgeted" | "full";

export interface WorkerSchedule {
  enabled: boolean;
  plan: When;
  report: When;
  timezone?: string | null;
  last_plan?: number | null;
  last_report?: number | null;
}

export interface Worker {
  tenant: string;
  agent: string;
  role: string;
  department: string;
  /** Who it answers to: another worker, or you (none). */
  reports_to: string | null;
  manager: boolean;
  autonomy: AutonomyLevel;
  weekly_budget_micros: number | null;
  task_time_ms: number;
  still_ask: string[];
  schedule: WorkerSchedule;
  revision: number;
  hired_at: number;
  updated_at: number;
}

export interface Department {
  slug: string;
  name: string;
  description: string;
  lead: string | null;
}

export interface OrgSettings {
  weekly_budget_micros: number | null;
  default_level: AutonomyLevel;
  departments: Department[];
  seen_until?: number | null;
}

export interface Figures {
  tasks: number;
  on_time_done: number;
  on_time_all: number;
  eval: number | null;
  week_micros: number;
  waiting: number;
}

export type WorkerStatus = "ok" | "warn" | "err";

export interface WorkerView {
  worker: Worker;
  name: string;
  description: string;
  avatar: { emoji: string; color: string };
  status: WorkerStatus | null;
  /** For a worker with reports, everyone below it too. */
  figures: Figures;
  own: Figures;
  reports: string[];
  budget_left_micros: number | null;
  archived: boolean;
}

export interface YouView {
  to_review: number;
  approvals: number;
  reports: string[];
}

export interface Overview {
  settings: OrgSettings;
  workers: WorkerView[];
  you: YouView;
  open_tasks: number;
  due_this_week: number;
  waiting_approvals: number;
  waiting_reviews: number;
  week_micros: number;
  budget_micros: number | null;
  running: number;
  busy: number;
  week: string;
}

export type ActivityLevel = "OK" | "INFO" | "WAIT" | "WARN" | "ERR";

export interface ActivityLine {
  id: string;
  at: number;
  level: ActivityLevel;
  source: string;
  text: string;
  kind: "suggestion" | "unattended" | "waiting" | "failed" | "plan" | "report" | "budget" | "org";
  run: string | null;
}

export interface Notice {
  id: string;
  kind: "suggestion" | "notice" | "report";
  at: number;
  worker: string | null;
  title: string;
  text: string;
  run: string | null;
}

export type ReportKind = "plan" | "report";
export type ReportStatus = "running" | "done" | "failed";

export interface OrgReport {
  id: string;
  worker: string;
  kind: ReportKind;
  week: string;
  run: string;
  status: ReportStatus;
  body: string;
  error?: string | null;
  by: string;
  started_at: number;
  finished_at?: number | null;
}

export interface OpenTask {
  key: string;
  title: string;
  state: TaskState;
  due: string | null;
  late: boolean;
}

export interface WorkerDetail {
  view: WorkerView;
  spend_days: number[];
  spend_last_week_micros: number;
  tasks: OpenTask[];
  activity: ActivityLine[];
  reports: OrgReport[];
  busy_with: ReportKind | null;
}

export interface NewWorkerRequest {
  agent: string;
  role: string;
  department: string;
  reports_to: string | null;
  manager: boolean;
  autonomy: AutonomyLevel | null;
  weekly_budget_micros: number | null;
  task_time_ms: number | null;
  still_ask: string[];
}

export interface WorkerFields {
  role: string;
  department: string;
  reports_to: string | null;
  manager: boolean;
  autonomy: AutonomyLevel;
  weekly_budget_micros: number | null;
  task_time_ms: number;
  still_ask: string[];
  schedule_enabled: boolean;
}

export interface SettingsFields {
  weekly_budget_micros: number | null;
  default_level: AutonomyLevel;
  departments: Department[];
}

const fillDepartment = (d: Department): Department => ({ ...d, description: d.description ?? "", lead: d.lead ?? null });
const fillSettings = (s: OrgSettings): OrgSettings => ({
  ...s,
  weekly_budget_micros: s.weekly_budget_micros ?? null,
  departments: s.departments.map(fillDepartment),
});
const fillWorker = (w: Worker): Worker => ({
  ...w,
  reports_to: w.reports_to ?? null,
  weekly_budget_micros: w.weekly_budget_micros ?? null,
  still_ask: w.still_ask ?? [],
});
const fillFigures = (f: Figures): Figures => ({ ...f, eval: f.eval ?? null });
const fillWorkerView = (v: WorkerView): WorkerView => ({
  ...v,
  worker: fillWorker(v.worker),
  status: v.status ?? null,
  figures: fillFigures(v.figures),
  own: fillFigures(v.own),
  budget_left_micros: v.budget_left_micros ?? null,
});
const fillActivity = (l: ActivityLine): ActivityLine => ({ ...l, run: l.run ?? null });

export const workforceOverview = () =>
  invoke<Overview>("workforce_overview").then((o) => ({ ...o, settings: fillSettings(o.settings), budget_micros: o.budget_micros ?? null, workers: o.workers.map(fillWorkerView) }));
export const workerDetail = (agent: string) =>
  invoke<WorkerDetail>("worker_detail", { agent }).then((d) => ({
    ...d,
    view: fillWorkerView(d.view),
    activity: d.activity.map(fillActivity),
    busy_with: d.busy_with ?? null,
    tasks: d.tasks.map((t) => ({ ...t, due: t.due ?? null })),
  }));
export const workerHire = (request: NewWorkerRequest) => invoke<Worker>("worker_hire", { new: request }).then(fillWorker);
export const workerEdit = (agent: string, fields: WorkerFields) => invoke<Worker>("worker_edit", { agent, fields }).then(fillWorker);
export const workerFire = (agent: string) => invoke<void>("worker_fire", { agent });
export const workforceSettingsSave = (fields: SettingsFields) => invoke<OrgSettings>("workforce_settings_save", { fields }).then(fillSettings);
export const workforceActivity = (limit = 100) => invoke<ActivityLine[]>("workforce_activity", { limit }).then((l) => l.map(fillActivity));
export const workforceNotices = () => invoke<Notice[]>("workforce_notices").then((n) => n.map((x) => ({ ...x, run: x.run ?? null })));
export const workforceMarkSeen = () => invoke<void>("workforce_mark_seen");
export const workerPlan = (agent: string, prompt?: string) => invoke<OrgReport>("worker_plan", { agent, prompt: prompt ?? null });
export const workerReport = (agent: string, prompt?: string) => invoke<OrgReport>("worker_report", { agent, prompt: prompt ?? null });
