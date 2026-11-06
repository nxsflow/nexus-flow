import { test } from "vitest"; import { spawn } from "node:child_process";
test("a worker that starts a detached child and waits", async () => {
  const c = spawn("node", ["-e", "/*TV58MARK-vitest-detached*/ setInterval(()=>{const t=Date.now();while(Date.now()-t<100);},3000)"], { detached: true, stdio: "ignore" }); c.unref();
  await new Promise((r) => setTimeout(r, 840000)); });
