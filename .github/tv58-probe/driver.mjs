// Throwaway (tv58 step 0): what the facts-mode sidecar will be, without the facts: one SDK session, its stream recorded.
import { appendFileSync, writeFileSync } from "node:fs";
const [sdkPath, dir, prompt, mcp] = process.argv.slice(2);
const { query } = await import(sdkPath);
const log = (o) => appendFileSync(`${dir}/sdk.jsonl`, JSON.stringify({ at: Date.now(), ...o }) + "\n");
const stop = new AbortController();
process.on("SIGTERM", () => { log({ driver: "SIGTERM" }); stop.abort(); });
process.on("SIGUSR2", () => log({ driver: "SIGUSR2" }));
async function* input() {
  yield { type: "user", session_id: "", parent_tool_use_id: null, message: { role: "user", content: prompt } };
  await new Promise((r) => stop.signal.addEventListener("abort", r)); // the input stays open, as the sidecar's does
}
if (process.env.PROBE_CLOSE_AT_RESULT) { /* closes the input at the first result, as the sidecar does */ }
const options = { abortController: stop, cwd: dir, ...(process.env.PROBE_PERM === "default" ? { permissionMode: "default" } : { permissionMode: "bypassPermissions", allowDangerouslySkipPermissions: true }),
  includePartialMessages: true, forwardSubagentText: true, settingSources: [], model: process.env.PROBE_MODEL || "claude-haiku-4-5",
  env: process.env, stderr: (s) => log({ stderr: s.slice(0, 300) }) };
if (mcp) options.mcpServers = { burner: { type: "stdio", command: "node", args: [mcp] } };
log({ driver: "query" });
try { for await (const m of query({ prompt: input(), options })) { log({ m }); } log({ driver: "stream ended" }); }
catch (e) { log({ driver: "threw", e: String(e) }); }
log({ driver: "exit" }); process.exit(0);
