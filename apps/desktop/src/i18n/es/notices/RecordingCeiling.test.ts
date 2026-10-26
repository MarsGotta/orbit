import { expect, it } from "vitest";
import { receive } from "../../../app/interceptor";

it("accepts retired ceiling notices from old Macs silently", () => {
  for (const code of [
    "recording_ceiling_near",
    "recording_ceiling_reached",
    "call_ceiling_near",
    "call_expired",
  ] as const) {
    const result = receive([], {
      kind: "notice",
      notice: { code, op: "retired-ceiling", data: { remaining: 10, minutes: 230, max: 240 } },
    });
    expect(result.messages).toEqual([]);
    expect(result.report).toBeNull();
  }
});
