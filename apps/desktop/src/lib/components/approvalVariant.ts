import { t } from "../i18n/index.ts";
// Which layout an approval gets. A tool's id says what kind of thing it does; each kind shows what a person needs to decide.
export type ApprovalVariant = "remember" | "mcp" | "http" | "webhook" | "git" | "jira" | "chat" | "db" | "code" | "generic";

export function approvalVariant(tool: string): ApprovalVariant {
  if (tool === "memory.remember") return "remember";
  if (tool.startsWith("mcp.")) return "mcp";
  if (tool === "http.request") return "http";
  if (tool === "webhook.send") return "webhook";
  if (tool.startsWith("github.") || tool.startsWith("gitlab.")) return "git";
  if (tool.startsWith("jira.")) return "jira";
  if (tool.startsWith("slack.") || tool.startsWith("discord.")) return "chat";
  if (tool === "db.execute") return "db";
  if (tool.startsWith("code.")) return "code";
  return "generic";
}

/** `mcp.github.create_issue` is the tool `create_issue` of the server `github`. Server names have no dots; tool names may. */
export function splitMcpTool(id: string): { server: string; tool: string } | null {
  const match = /^mcp\.([a-z0-9_-]+)\.(.+)$/.exec(id);
  return match ? { server: match[1], tool: match[2] } : null;
}

/** `github.create_issue` is the action `create_issue` of the service `github`. */
export function splitService(id: string): { service: string; action: string } {
  const dot = id.indexOf(".");
  return dot < 0 ? { service: id, action: "" } : { service: id.slice(0, dot), action: id.slice(dot + 1) };
}

const WORDS: Record<string, string> = {
  create_issue: t("approvalVariant.openAnIssue"),
  update_issue: t("approvalVariant.changeAnIssue"),
  comment: t("approvalVariant.addAComment"),
  send_message: t("approvalVariant.sendAMessage"),
  transition: t("approvalVariant.moveAnIssue"),
  execute: t("approvalVariant.changeData"),
};

export function actionWords(action: string): string {
  return WORDS[action] ?? action.replaceAll("_", " ");
}
