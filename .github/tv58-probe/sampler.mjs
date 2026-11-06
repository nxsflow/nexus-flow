// Throwaway: one raw process table per interval, everything the analysis needs. Linux CPU in ticks from /proc, macOS `time`.
import { execFileSync } from "node:child_process"; import { appendFileSync, readFileSync } from "node:fs";
const [out, every, count] = process.argv.slice(2);
const linux = process.platform === "linux";
const cpuOf = (pid, fallback) => { if (!linux) return fallback; try { const s = readFileSync(`/proc/${pid}/stat`, "utf8"); const r = s.slice(s.lastIndexOf(")") + 2).split(" "); return String(Number(r[11]) + Number(r[12])); } catch { return "gone"; } };
for (let i = 0; i < Number(count); i++) {
  const text = execFileSync("ps", ["-A", "-ww", "-o", "pid=,ppid=,pgid=,sess=,stat=,lstart=,time=,command="], { encoding: "utf8", maxBuffer: 1 << 26 });
  const rows = text.split("\n").filter(Boolean).map((l) => { const m = l.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(\S+ \S+\s+\d+ \S+ \d+)\s+(\S+)\s+(.*)$/); if (!m) return null; return { pid: +m[1], ppid: +m[2], pgid: +m[3], sess: m[4], stat: m[5], start: Date.parse(m[6]), cpu: cpuOf(+m[1], m[7]), cmd: m[8].slice(0, 160) }; }).filter(Boolean);
  appendFileSync(out, JSON.stringify({ at: Date.now(), rows }) + "\n");
  await new Promise((r) => setTimeout(r, Number(every) * 1000));
}
