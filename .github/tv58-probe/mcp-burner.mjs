// Throwaway: a stdio MCP server that spends CPU for ever; TV58MARK-mcp is in the command line through the file name's dir.
import { createInterface } from "node:readline";
setInterval(() => { const t = Date.now(); while (Date.now() - t < 200); }, 2000);
createInterface({ input: process.stdin }).on("line", (l) => {
  const q = JSON.parse(l); if (q.id === undefined) return;
  const result = q.method === "initialize" ? { protocolVersion: q.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "burner", version: "1" } } : { tools: [] };
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: q.id, result }) + "\n");
});
