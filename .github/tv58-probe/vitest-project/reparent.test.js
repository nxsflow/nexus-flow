import { test } from "vitest"; import { execSync } from "node:child_process";
test("a worker whose grandchild reparents", async () => {
  execSync(`sh -c 'node -e "/*TV58MARK-vitest-reparented*/ setInterval(()=>{const t=Date.now();while(Date.now()-t<100);},3000)" >/dev/null 2>&1 &'`);
  await new Promise((r) => setTimeout(r, 840000)); });
