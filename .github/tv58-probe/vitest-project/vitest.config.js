import { defineConfig } from "vitest/config";
export default defineConfig({ test: { pool: "forks", testTimeout: 900000, include: ["*.test.js"] } });
