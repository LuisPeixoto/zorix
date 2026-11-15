// The only file that talks to the Rust side. Types mirror what the engine sends (serde JSON).
import { Channel, invoke } from "@tauri-apps/api/core";

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
}

export interface AppInfo {
  version: string;
  data_dir: string;
  error: string | null;
}

export interface StartRun {
  prompt: string;
  instructions?: string;
  allow_tools: string[];
  allow_read: string[];
  allow_write: string[];
  workspace?: string;
  max_iterations?: number;
}

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
