import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { CallSnapshot } from "../bridge/generated/CallSnapshot";
import { CallCard } from "./CallCard";

function call(change: Partial<CallSnapshot> = {}): CallSnapshot {
  return {
    started_at_ms: Date.now() - 252_000,
    seeing_screen: true,
    last_line: null,
    speaking: false,
    expires_at_ms: Date.now() + 55 * 60_000,
    origin: "app",
    idle_hangup_ms: 60_000,
    ...change,
  };
}

function renderCard(value = call(), agentName = "Deimos") {
  const sent: AppCommand[] = [];
  render(<CallCard agentName={agentName} call={value} send={(command) => sent.push(command)} />);
  return sent;
}

describe("the call card", () => {
  it("says who you talk to, for how long, what it sees and what is left", () => {
    renderCard();
    const card = screen.getByRole("region", { name: "Llamada en curso" });
    expect(card.textContent).toContain("En llamada");
    expect(screen.getByRole("timer", { name: "Tiempo de llamada" }).textContent).toBe("04:12");
    expect(card.textContent).toContain("Deimos");
    expect(card.textContent).toContain("Ve tu pantalla · quedan 55 min");
  });

  it("says the hours left of a call that has them", () => {
    renderCard(call({ expires_at_ms: Date.now() + 239 * 60_000 + 30_000 }));
    const card = screen.getByRole("region", { name: "Llamada en curso" });
    expect(card.textContent).toContain("Ve tu pantalla · quedan 3 h 59 min");
  });

  it("names the agent of the call and what it cannot see", () => {
    renderCard(call({ seeing_screen: false, expires_at_ms: Date.now() + 30_000 }), "Vega");
    const card = screen.getByRole("region", { name: "Llamada en curso" });
    expect(card.textContent).toContain("Vega");
    expect(card.textContent).toContain("No ve tu pantalla · queda 1 min");
  });

  it("«Colgar» ends the call", async () => {
    const sent = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Colgar" }));
    expect(sent).toEqual([{ command: "call_stop" }]);
  });
});
