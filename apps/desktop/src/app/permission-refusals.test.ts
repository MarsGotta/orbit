import { expect, it } from "vitest";
import { receive } from "./interceptor";

it("names the calendar permission as Rust does and opens its pane", () => {
  const result = receive([], {
    kind: "refusal",
    op: "calendar-permission",
    refusal: { code: "permission_denied", permission: "calendar", retry: "never" },
  });
  expect(result.messages[0]?.text).toBe(
    "Falta el permiso de calendario. Concédelo en Ajustes del Sistema.",
  );
  expect(result.messages[0]?.action).toEqual({
    label: "Abrir Ajustes del Sistema",
    command: { command: "open_system_privacy", pane: "Calendars" },
  });
});

it("names automation and leads to Ajustes without asking for a pane Tauri does not support", () => {
  const result = receive([], {
    kind: "refusal",
    op: "automation-permission",
    refusal: { code: "permission_denied", permission: "automation", retry: "never" },
  });
  expect(result.messages[0]?.text).toBe(
    "Falta el permiso de automatización. Concédelo en Ajustes del Sistema.",
  );
  expect(result.messages[0]?.action).toEqual({ label: "Abrir Ajustes", view: "settings" });
});
