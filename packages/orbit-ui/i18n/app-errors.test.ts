import { describe, expect, it } from "vitest";
import { appOperationText as es } from "./es/errors";
import { appOperationText as en } from "./en/errors";
import type { Refusal } from "../bridge/refusal";

function operation(code: string, error = "private diagnostic"): Refusal {
  return {
    code: "internal",
    retry: { kind: "never" },
    cause: { kind: "app_operation", code, error },
  };
}

describe("desktop operation error codes", () => {
  it("uses catalogue text in both languages and keeps diagnostic text private", () => {
    expect(es(operation("link_scheme"))).toContain("http y https");
    expect(en(operation("link_scheme"))).toContain("http and https");
    expect(es(operation("note_unavailable"))).toContain("nota");
    expect(en(operation("note_unavailable"))).toContain("note");
    expect(en(operation("link_credentials"))).not.toContain("private diagnostic");
  });
  it("preserves event counts and leaves unknown codes to the generic failure catalogue", () => {
    expect(es(operation("meeting_event_not_found", JSON.stringify({ count: 17 })))).toContain(
      "17 eventos",
    );
    expect(en(operation("meeting_event_not_found", JSON.stringify({ count: 17 })))).toContain(
      "17 loaded events",
    );
    expect(en(operation("future_error"))).toBeUndefined();
    expect(es(operation("future_error"))).toBeUndefined();
  });
});
