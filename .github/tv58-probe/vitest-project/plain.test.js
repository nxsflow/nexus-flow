import { test } from "vitest";
test("a worker that works", async () => { const end = Date.now() + 840000; while (Date.now() < end) { const t = Date.now(); while (Date.now() - t < 100); await new Promise((r) => setTimeout(r, 3000)); } });
