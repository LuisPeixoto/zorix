// A tiny MCP server for the container tests (ADR-0050): its one tool, `probe`, tries what its arguments name and says how each went.
const fs = require("fs");
const net = require("net");
const readline = require("readline");

const send = (message) => process.stdout.write(JSON.stringify(message) + "\n");
const word = (ok) => (ok ? "ok" : "denied");
const attempt = (work) => {
  try {
    work();
    return true;
  } catch {
    return false;
  }
};
const connect = (address) =>
  new Promise((resolve) => {
    const [host, port] = address.split(":");
    const socket = net.connect({ host, port: Number(port), timeout: 2000 });
    const done = (ok) => {
      socket.destroy();
      resolve(ok);
    };
    socket.on("connect", () => done(true));
    socket.on("error", () => done(false));
    socket.on("timeout", () => done(false));
  });

async function probe(args) {
  const out = [];
  for (const path of args.read || []) out.push(`read ${path}=${word(attempt(() => fs.readFileSync(path)))}`);
  for (const path of args.write || []) out.push(`write ${path}=${word(attempt(() => fs.writeFileSync(path, "x")))}`);
  for (const address of args.connect || []) out.push(`connect ${address}=${word(await connect(address))}`);
  out.push(`home=${process.env.HOME}`);
  out.push(`given=${process.env.ZORIX_PROBE || ""}`);
  return out.join("\n");
}

readline.createInterface({ input: process.stdin }).on("line", async (line) => {
  const { id, method, params } = JSON.parse(line);
  if (method === "initialize") {
    send({ jsonrpc: "2.0", id, result: { protocolVersion: "2025-11-25", capabilities: { tools: {} }, serverInfo: { name: "probe", version: "1" } } });
  } else if (method === "tools/list") {
    send({ jsonrpc: "2.0", id, result: { tools: [{ name: "probe", inputSchema: { type: "object" } }] } });
  } else if (method === "tools/call") {
    send({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text: await probe(params.arguments || {}) }] } });
  } else if (id !== undefined) {
    send({ jsonrpc: "2.0", id, error: { code: -32601, message: "Method not found" } });
  }
});
