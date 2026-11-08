import { afterEach, describe, expect, it } from "vitest";
import { receive } from "../../../state/interceptor";
import type { Notice } from "../../../bridge/generated/Notice";
import { configureCopy } from "../..";
import { hubRefusalText as englishRefusal } from "../../en/hubRefusals";
import { hubRefusalText } from "../hubRefusals";

// A hub without Linux has no jail for Claude Code or Codex: the person reads
// that cause, in the chat and in the assistant's settings, never a generic
// failure, and never advice to publish or install anything.

const SPANISH =
  "Claude y Codex solo funcionan en un hub con Linux. En este hub, elige otro asistente.";
const ENGLISH =
  "Claude and Codex only work on a hub running Linux. On this hub, choose another assistant.";
const OP = "5c1d2e3f-0000-4000-8000-000000000007";

function shown(): string | undefined {
  const notice: Notice = {
    code: "assistant_failed",
    op: OP,
    data: { code: "assistant_jail_unavailable" },
  };
  return receive([], { kind: "notice", notice }).messages[0]?.text;
}

afterEach(() => configureCopy("es", ["es-ES"], "Atlas"));

describe("an official assistant on a hub without a jail", () => {
  it("says the cause in the chat, in both languages", () => {
    configureCopy("es", ["es-ES"], "Atlas");
    expect(shown()).toBe(SPANISH);
    configureCopy("en", ["en"], "Atlas");
    expect(shown()).toBe(ENGLISH);
  });

  it("says the same as a refusal of the hub", () => {
    expect(hubRefusalText("assistant_jail_unavailable", { service: "claude-code" })).toBe(
      `${SPANISH} (service: claude-code)`,
    );
    expect(englishRefusal("assistant_jail_unavailable", { service: "claude-code" })).toBe(
      `${ENGLISH} (service: claude-code)`,
    );
  });
});
