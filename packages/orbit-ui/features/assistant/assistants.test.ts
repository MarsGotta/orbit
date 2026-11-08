import { describe, expect, it } from "vitest";
import type { Agent } from "../../bridge/generated/Agent";
import {
  colorOf,
  composerBlock,
  healthOf,
  nameKey,
  profileProblem,
  rowWarning,
  statusLine,
} from "./assistants";

// Wednesday 21 October 2026, 12:00 on the Mac's clock.
const NOW = new Date(2026, 9, 21, 12, 0).getTime();
const AT_1840 = new Date(2026, 9, 21, 18, 40).getTime();

const deimos: Agent = { id: "deimos", name: "Deimos", service: "hermes", color: "c1" };
const phobos: Agent = {
  id: "codex-1a2b3c4d",
  name: "Phobos",
  service: "codex",
  color: "c2",
  account: "signed_in",
  removable: true,
};
const ares: Agent = {
  id: "claude-5e6f7a8b",
  name: "Ares",
  service: "claude-code",
  color: "c3",
  account: "expired",
  removable: true,
};

describe("an assistant's health", () => {
  it("needs a sign-in when its session expired or never began, naming the vendor", () => {
    expect(healthOf(ares, NOW)).toEqual({ kind: "sign_in", reason: "expired", vendor: "Claude" });
    expect(healthOf({ ...phobos, account: "signed_out" }, NOW)).toEqual({
      kind: "sign_in",
      reason: "signed_out",
      vendor: "ChatGPT",
    });
  });

  it("is at its limit until the time the provider gave, and over once it passed", () => {
    const limited = { ...phobos, limit: { until_ms: AT_1840 } };
    expect(healthOf(limited, NOW)).toEqual({ kind: "limit", until: AT_1840 });
    expect(healthOf(limited, AT_1840 + 1)).toEqual({ kind: "ok" });
    // Without a time the limit holds: not saying is not «no limit».
    expect(healthOf({ ...phobos, limit: {} }, NOW)).toEqual({ kind: "limit" });
  });

  it("is unknown while the hub has not learned its account, and fine without one to sign", () => {
    expect(healthOf({ ...phobos, account: "unknown" }, NOW)).toEqual({ kind: "unknown" });
    expect(healthOf(deimos, NOW)).toEqual({ kind: "ok" });
  });
});

describe("what «Tus asistentes» and the cards say", () => {
  it("warns on the row only when something is wrong", () => {
    expect(rowWarning(ares, NOW)).toBe("Inicia sesión");
    expect(rowWarning({ ...phobos, limit: { until_ms: AT_1840 } }, NOW)).toBe("Hasta 18:40");
    expect(rowWarning({ ...phobos, limit: {} }, NOW)).toBe("En su límite");
    expect(rowWarning(phobos, NOW)).toBeNull();
  });

  it("says the status with the plan's usage only when the provider gave it", () => {
    expect(statusLine(deimos, NOW)).toEqual({ tone: "ok", text: "Disponible · con tu hub" });
    expect(statusLine(phobos, NOW)).toEqual({ tone: "ok", text: "Disponible" });
    expect(
      statusLine({ ...phobos, usage: { used_percent: 72, window: "seven_day" } }, NOW),
    ).toEqual({ tone: "ok", text: "Disponible · 72 % del uso de la semana" });
    expect(statusLine(ares, NOW)).toEqual({ tone: "warn", text: "La sesión de Claude caducó" });
    expect(statusLine({ ...phobos, limit: { until_ms: AT_1840 } }, NOW)).toEqual({
      tone: "warn",
      text: "En su límite hasta hoy a las 18:40",
    });
    expect(statusLine({ ...phobos, account: "unknown" }, NOW)).toBeNull();
  });

  it("keeps the writing and says why it cannot be sent", () => {
    expect(composerBlock(ares, NOW)).toBe(
      "Ares no puede responder hasta que inicies sesión en Claude. Lo que escribas se queda aquí.",
    );
    expect(composerBlock({ ...phobos, limit: { until_ms: AT_1840 } }, NOW)).toBe(
      "Phobos no puede responder hasta las 18:40. Lo que escribas se queda aquí.",
    );
    expect(composerBlock(phobos, NOW)).toBeNull();
    expect(composerBlock(undefined, NOW)).toBeNull();
  });
});

describe("an assistant's colour", () => {
  it("is the one it has, or the four in turn by its position", () => {
    expect(colorOf(phobos, [deimos, phobos])).toBe(2);
    const plain = [
      { id: "a", name: "A" },
      { id: "b", name: "B" },
      { id: "c", name: "C" },
      { id: "d", name: "D" },
      { id: "e", name: "E" },
    ];
    expect(plain.map((agent) => colorOf(agent, plain))).toEqual([1, 2, 3, 4, 1]);
  });
});

describe("the name in the pencil", () => {
  const all = [deimos, phobos, ares];

  it("compares names as the wake phrase hears them: no case, no accents", () => {
    expect(nameKey("  Árès ")).toBe("ares");
  });

  it("refuses an empty name and one another assistant has, and keeps its own", () => {
    expect(profileProblem("   ", phobos.id, all)).toBe("empty");
    expect(profileProblem("áres", phobos.id, all)).toBe("taken");
    expect(profileProblem("Phobos", phobos.id, all)).toBeNull();
    expect(profileProblem("Eris", phobos.id, all)).toBeNull();
  });

  it("refuses more than 24 characters", () => {
    expect(profileProblem("x".repeat(25), phobos.id, all)).toBe("too_long");
    expect(profileProblem("x".repeat(24), phobos.id, all)).toBeNull();
  });
});
