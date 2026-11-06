// A sidecar with no SDK (§12.8): it takes the flags of the real one, writes `child.json` first and then does what the
// spec's turn text says, one step at a time. Every wait is on a file in its ledger directory, so a test decides when
// anything happens and nothing rests on the clock.
import { spawn } from "node:child_process";
import { appendFileSync, existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const specPath = process.argv[process.argv.indexOf("--spec") + 1];
const spec = JSON.parse(readFileSync(specPath, "utf8"));
const ledger = spec.ledger_dir;
const script = JSON.parse(spec.turn.text);

let ignoreTerm = script.ignore_term === true;
process.on("SIGTERM", () => {
  appendFileSync(join(ledger, "signals"), "TERM\n");
  if (ignoreTerm) return;
  process.exit(143);
});
process.on("SIGUSR2", () => appendFileSync(join(ledger, "signals"), "USR2\n"));

if (!script.no_child_json) {
  const temp = join(ledger, "child.json.tmp");
  const started = new Date(Date.now() - process.uptime() * 1000).toISOString();
  writeFileSync(temp, JSON.stringify({ pid: process.pid, start_time: started, handlers: true }));
  renameSync(temp, join(ledger, "child.json"));
}

const fact = (line) => appendFileSync(join(ledger, "facts.jsonl"), `${JSON.stringify(line)}\n`);
const untilFile = async (name) => {
  while (!existsSync(join(ledger, name))) await new Promise((r) => setTimeout(r, 5));
};

for (const step of script.steps ?? []) {
  if (step.fact) fact(step.fact);
  if (step.raw) appendFileSync(join(ledger, "facts.jsonl"), step.raw);
  if (step.wait) await untilFile(step.wait);
  if (step.spawn) {
    // A child of this process, in this process group, that leaves its pid where the test can read it.
    const child = spawn("sh", ["-c", step.spawn], { stdio: "ignore", cwd: ledger });
    writeFileSync(join(ledger, step.pid_file ?? "spawned"), String(child.pid));
    child.on("error", () => {});
  }
  if (step.cwd_to) writeFileSync(join(ledger, step.cwd_to), process.cwd());
  if (step.exit !== undefined) process.exit(step.exit);
}
// Waits for its signal, or for the test to end it.
setInterval(() => {}, 1000);
