import { describe, expect, it } from "vitest";
import { notices } from ".";

describe("audio the hub cannot read", () => {
  it("says its cause with how many groups of how many, as something to know", () => {
    const entry = notices.meeting_audio_unreadable;
    expect(entry.class).toBe("fixable");
    expect(entry.text({ count: 1, total: 4 })).toBe(
      "No se puede rehacer la transcripción: el audio de 1 de 4 grupos no se puede leer.",
    );
  });
});
