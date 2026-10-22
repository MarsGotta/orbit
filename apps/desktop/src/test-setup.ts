import { cleanup, configure } from "@testing-library/react";
import { afterEach, beforeEach } from "vitest";
import { QUERY_BUDGET_MS } from "./test-budgets";
import { TEST_NOW_MS } from "./test-clock";

// A query waits for what it asks about for as long as a loaded machine
// takes to render it, which is more than Testing Library's default second.
// The number and the measurement it comes from are in `test-budgets.ts`.
configure({ asyncUtilTimeout: QUERY_BUDGET_MS });

// Unmount what a test rendered, before the next one runs.
//
// Testing Library installs this itself, but only when it can see a global
// `afterEach` — and this suite runs without `globals`, so it never did. Every
// tree a test rendered therefore stayed mounted for the rest of its file, with
// its effects, its intervals and its daemon subscription all still live.
// Nothing failed over it, because those trees were never asked anything again.
// But a root that still had work scheduled when the environment was torn down
// flushed it into a world with no `window` left, and the run ended in
// `ReferenceError: window is not defined` with all 51 files reported passing.
afterEach(cleanup);

// Unsent drafts are kept in the webview's local storage (`state/local-drafts.ts`):
// one a test left must not appear in the next test's composer or panel.
afterEach(() => {
  try {
    for (const key of Object.keys(window.localStorage))
      if (key.startsWith("orbit.draft.")) window.localStorage.removeItem(key);
  } catch {
    // A test replaced the storage with one that refuses; it restores its own.
  }
});

// The unit suite reads a clock that does not move.
//
// Only the *reading* of the present instant is pinned: `setTimeout` and
// `setInterval` keep running for real, so user-event, React effects and the
// calendar's own one-minute refresh behave exactly as they did. Vitest's fake
// timers are deliberately not used for this, because three suites install
// their own (`vi.useFakeTimers` plus `advanceTimersByTime`) and a second
// installation on top of theirs leaves their timers unfaked. A proxy over the
// global `Date` is invisible to that machinery: when a suite installs fake
// timers it saves this proxy as the original and `vi.useRealTimers()` puts it
// back, so the pin survives its own teardown.
// The real constructor is kept on the global, so that re-evaluating this setup
// in a worker that already ran another file captures it and never a former pin.
const globals = globalThis as typeof globalThis & { orbitRealDate?: DateConstructor };
globals.orbitRealDate ??= globalThis.Date;
const realDate = globals.orbitRealDate;

function pinDate(): void {
  globalThis.Date = new Proxy(realDate, {
    construct: (target, args) =>
      args.length === 0 ? new target(TEST_NOW_MS) : Reflect.construct(target, args),
    apply: (target) => new target(TEST_NOW_MS).toString(),
    get: (target, property, receiver) =>
      property === "now" ? () => TEST_NOW_MS : Reflect.get(target, property, receiver),
  });
}

pinDate();
beforeEach(pinDate);

// The Markdown editor (CodeMirror) measures the text and the cursor to
// place them and to scroll the cursor into view. jsdom lays nothing out and has no
// `Range.getClientRects`; WebKit has. An empty layout is what jsdom would
// report for everything else, so the editor sees nothing to scroll.
if (!("getClientRects" in Range.prototype)) {
  Object.defineProperty(Range.prototype, "getClientRects", {
    configurable: true,
    value: () => ({ length: 0, item: () => null, [Symbol.iterator]: [][Symbol.iterator] }),
  });
}
if (!("getBoundingClientRect" in Range.prototype)) {
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => new DOMRect(),
  });
}
