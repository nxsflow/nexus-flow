// A local Messages API for `tests/real_sidecar.rs`: the REAL CLI runs REAL tools with no credentials. A first message holding
// `RUN::{"name":"Bash","input":{...}}` gets that tool call as its answer, and everything else gets "done".
import { createServer } from "node:http";
import { writeFileSync } from "node:fs";
const sse = (res, ev, d) => res.write(`event: ${ev}\ndata: ${JSON.stringify({ type: ev, ...d })}\n\n`);
function textOf(m) { return typeof m.content === "string" ? m.content : (m.content ?? []).map((c) => c.text ?? "").join(""); }
function reply(res, model, blocks, stop) {
  res.writeHead(200, { "content-type": "text/event-stream" });
  sse(res, "message_start", { message: { id: `msg_${Date.now()}`, type: "message", role: "assistant", model, content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 1, output_tokens: 1 } } });
  blocks.forEach((b, index) => {
    if (b.type === "text") {
      sse(res, "content_block_start", { index, content_block: { type: "text", text: "" } });
      sse(res, "content_block_delta", { index, delta: { type: "text_delta", text: b.text } });
    } else {
      sse(res, "content_block_start", { index, content_block: { type: "tool_use", id: `toolu_${Date.now()}_${index}`, name: b.name, input: {} } });
      sse(res, "content_block_delta", { index, delta: { type: "input_json_delta", partial_json: JSON.stringify(b.input) } });
    }
    sse(res, "content_block_stop", { index });
  });
  sse(res, "message_delta", { delta: { stop_reason: stop, stop_sequence: null }, usage: { output_tokens: 1 } });
  sse(res, "message_stop", {});
  res.end();
}
const server = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (!req.url.startsWith("/v1/messages") || req.url.includes("count_tokens")) {
      res.writeHead(200, { "content-type": "application/json" });
      return res.end(req.url.includes("count_tokens") ? '{"input_tokens":1}' : "{}");
    }
    const q = JSON.parse(body);
    const first = textOf(q.messages[0]);
    const last = q.messages[q.messages.length - 1];
    const answered = Array.isArray(last.content) && last.content.some((c) => c.type === "tool_result");
    const at = first.indexOf("RUN::");
    if (at >= 0 && !answered && q.messages.length === 1) {
      const { name = "Bash", input } = JSON.parse(first.slice(at + 5).split("\n")[0]);
      return reply(res, q.model, [{ type: "tool_use", name, input }], "tool_use");
    }
    return reply(res, q.model, [{ type: "text", text: "done" }], "end_turn");
  });
});
server.listen(0, "127.0.0.1", () => writeFileSync(process.argv[2], String(server.address().port)));
