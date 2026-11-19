import assert from "node:assert/strict";
import { test } from "node:test";

import { actionWords, approvalVariant, splitMcpTool, splitService } from "./approvalVariant.ts";

test("each family of tools gets its own layout", () => {
  const cases: Record<string, string> = {
    "memory.remember": "remember",
    "mcp.github.create_issue": "mcp",
    "http.request": "http",
    "webhook.send": "webhook",
    "github.comment": "git",
    "gitlab.update_issue": "git",
    "jira.transition": "jira",
    "slack.send_message": "chat",
    "discord.send_message": "chat",
    "db.execute": "db",
    "code.run_in_workspace": "code",
    "notes.put": "generic",
  };
  for (const [tool, want] of Object.entries(cases)) assert.equal(approvalVariant(tool), want, tool);
});

test("an MCP tool id splits into its server and tool, and tool names may contain dots", () => {
  assert.deepEqual(splitMcpTool("mcp.github.create_issue"), { server: "github", tool: "create_issue" });
  assert.deepEqual(splitMcpTool("mcp.self.memory.search"), { server: "self", tool: "memory.search" });
  assert.equal(splitMcpTool("github.comment"), null);
  assert.deepEqual(splitService("slack.send_message"), { service: "slack", action: "send_message" });
});

test("actions read as plain words", () => {
  assert.equal(actionWords("create_issue"), "open an issue");
  assert.equal(actionWords("list_pulls"), "list pulls");
});
