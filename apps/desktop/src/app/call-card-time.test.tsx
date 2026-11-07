import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { CallSnapshot } from "../bridge/generated/CallSnapshot";
import { CopyProvider } from "../i18n/CopyProvider";
import { CallCard } from "./CallCard";

// A call has no time limit: it ends when the person hangs up or when nobody
// answers. The engine still sends `expires_at_ms`, a retired field: new calls
// carry i64::MAX and nothing hangs up at any value it holds. The card shows
// how long the call has lasted, never how long it has left.

// The retired field exactly as the engine writes it for every new call.
const RETIRED_DEADLINE: number = JSON.parse('{"expires_at_ms":9223372036854775807}').expires_at_ms;

const deadlines = [
  ["the value every new call carries", () => RETIRED_DEADLINE],
  ["a deadline 55 minutes away", () => Date.now() + 55 * 60_000],
  ["a deadline 30 seconds away", () => Date.now() + 30_000],
] as const;

function call(change: Partial<CallSnapshot> = {}): CallSnapshot {
  return {
    started_at_ms: Date.now() - 252_000,
    seeing_screen: true,
    last_line: null,
    speaking: false,
    expires_at_ms: RETIRED_DEADLINE,
    origin: "hotkey",
    idle_hangup_ms: 60_000,
    ...change,
  };
}

function renderCard(value: CallSnapshot, language: "es" | "en") {
  render(
    <CopyProvider choice={language}>
      <CallCard agentName="Vega" call={value} send={() => undefined} />
    </CopyProvider>,
  );
}

describe("the call card never counts down", () => {
  for (const [what, deadline] of deadlines) {
    it(`in Spanish, with ${what}, says what it sees and how long it has lasted`, () => {
      renderCard(call({ expires_at_ms: deadline() }), "es");
      const card = screen.getByRole("region", { name: "Llamada en curso" });
      expect(screen.getByText("Ve tu pantalla")).toBeTruthy();
      expect(screen.getByRole("timer", { name: "Tiempo de llamada" }).textContent).toBe("04:12");
      expect(card.textContent).not.toMatch(/queda|min\b| h\b/);
    });

    it(`in English, with ${what}, says what it sees and how long it has lasted`, () => {
      renderCard(call({ expires_at_ms: deadline() }), "en");
      const card = screen.getByRole("region", { name: "Call in progress" });
      expect(screen.getByText("Can see your screen")).toBeTruthy();
      expect(screen.getByRole("timer", { name: "Call duration" }).textContent).toBe("04:12");
      expect(card.textContent).not.toMatch(/left|min\b| h\b/);
    });
  }

  it("a call that cannot see the screen says only that, in both languages", () => {
    renderCard(call({ seeing_screen: false }), "es");
    expect(screen.getByText("No ve tu pantalla")).toBeTruthy();
    renderCard(call({ seeing_screen: false }), "en");
    expect(screen.getByText("Can’t see your screen")).toBeTruthy();
  });
});
