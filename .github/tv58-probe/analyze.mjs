// Throwaway: for each scenario and sample: who does each candidate filter see, and did each signature move.
import { readFileSync, existsSync } from "node:fs";
const [base, ...names] = process.argv.slice(2);
const samples = readFileSync(`${base}/samples.jsonl`, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
for (const name of names) {
  const dir = `${base}/${name}`; if (!existsSync(`${dir}/leader`)) continue;
  const leader = +readFileSync(`${dir}/leader`, "utf8"); console.log(`\n===== ${name}, leader ${leader}`);
  const sdk = readFileSync(`${dir}/sdk.jsonl`, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
  const init = sdk.find((e) => e.m?.type === "system" && e.m?.subtype === "init"), result = sdk.find((e) => e.m?.type === "result");
  const firstTool = sdk.find((e) => e.m?.type === "assistant" && e.m.message?.content?.some?.((c) => c.type === "tool_use"));
  console.log(`init at ${init?.at} result at ${result?.at} first tool at ${firstTool?.at} types: ${[...new Set(sdk.map((e) => e.m ? e.m.type + "/" + (e.m.subtype ?? "") : ""))].join(" ")}`);
  let prev = {}; const known = new Map();
  for (const s of samples) {
    const by = new Map(s.rows.map((r) => [r.pid, r])); const kids = new Map(); s.rows.forEach((r) => { if (!kids.has(r.ppid)) kids.set(r.ppid, []); kids.get(r.ppid).push(r); });
    const lin = []; const walk = (p) => (kids.get(p) ?? []).forEach((c) => { lin.push(c); walk(c.pid); }); walk(leader);
    const group = s.rows.filter((r) => r.pgid === leader && r.pid !== leader);
    const mark = s.rows.filter((r) => r.cmd.includes(`TV58MARK-${name}`) && !r.cmd.includes("spawner.pl") && !r.cmd.includes("driver.mjs"));
    const cli = lin.find((r) => r.ppid === leader);
    const cliKids = cli ? (kids.get(cli.pid) ?? []) : [];
    const sets = { lineage: lin, lineageNoCli: lin.filter((r) => r !== cli), group, groupNoCli: group.filter((r) => r !== cli) };
    if (init) { sets.afterInit = lin.filter((r) => r !== cli && r.start >= Math.floor(init.at / 1000) * 1000); sets.groupAfterInit = group.filter((r) => r !== cli && r.start >= Math.floor(init.at / 1000) * 1000); }
    const line = [];
    for (const [k, set] of Object.entries(sets)) {
      const cpu = set.map((r) => `${r.pid}:${r.cpu}`).sort().join(","); const pids = set.map((r) => r.pid).sort().join(",");
      const movedCpu = prev[k] && prev[k].cpu !== cpu; const movedPids = prev[k] && prev[k].pids !== pids; prev[k] = { cpu, pids };
      line.push(`${k}=${set.length}${movedCpu ? "*cpu" : ""}${movedPids ? "*who" : ""}`);
    }
    console.log(`t+${Math.round((s.at - samples[0].at) / 1000)}s ${line.join(" ")}`);
    console.log(`    cli=${cli ? cli.pid + ":" + cli.cpu : "-"} leader=${by.get(leader)?.cpu ?? "gone"} cliKids=${cliKids.map((r) => r.pid + ":" + r.cmd.slice(0, 40)).join(" | ")}`);
    for (const r of mark) { const inLin = lin.includes(r), inGrp = r.pgid === leader; const old = known.get(r.pid); const moved = old !== undefined && old !== r.cpu; known.set(r.pid, r.cpu); console.log(`    MARK pid=${r.pid} ppid=${r.ppid} pgid=${r.pgid} sess=${r.sess} stat=${r.stat} cpu=${r.cpu}${moved ? " MOVED" : ""} lineage=${inLin} group=${inGrp} ${r.cmd.slice(0, 80)}`); }
  }
  console.log("--- ladder"); console.log(readFileSync(`${dir}/ladder.log`, "utf8"));
}
