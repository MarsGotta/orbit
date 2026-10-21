import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { TEST_BUDGET_MS } from "./src/test-budgets";
import { TEST_TIMEZONE } from "./src/test-clock";

const EXCLUDED = ["node_modules/**", "dist/**"];
const DESIGN_TESTS = "src/design/**/*.test.{ts,tsx}";

export default defineConfig({
  clearScreen: false,
  plugins: [react()],
  server: {
    strictPort: true,
  },
  test: {
    // The zone is pinned too, so a run in another zone reads the same
    // labels. Node resets ICU when `process.env.TZ` is assigned, which is what
    // this does in every worker before the setup file runs.
    env: { TZ: TEST_TIMEZONE },
    environment: "jsdom",
    restoreMocks: true,
    setupFiles: ["./src/test-setup.ts"],
    // Derived from how long a test takes under load, not Vitest's
    // default 5 s, which was below the measured p99. See `src/test-budgets.ts`.
    testTimeout: TEST_BUDGET_MS,
    exclude: EXCLUDED,
    // Only the design system's tests read styles from the stylesheets, so only
    // they load them. With the stylesheets in, jsdom matches every rule's
    // selector for each style it computes, and nwsapi keeps compiled selectors
    // in an LRU of 1 000 (`CACHE_LIMIT`): once the app's rules passed that
    // (about 950 when this was measured), the cache missed on every match and
    // a paging test went from 11 s to 64 s on its own, past its budget. Measured
    // on 2026-10-15: the whole suite takes 149 s with the stylesheets and 17 s
    // without, and without them only these ten tests fail
    // (`src/design/{tokens,primitives}.test.tsx`; the second was removed since).
    projects: [
      {
        extends: true,
        test: { name: "design", include: [DESIGN_TESTS], css: true },
      },
      {
        extends: true,
        test: {
          name: "app",
          include: ["src/**/*.test.{ts,tsx}"],
          exclude: [...EXCLUDED, DESIGN_TESTS],
          css: { include: [] },
        },
      },
    ],
  },
});
