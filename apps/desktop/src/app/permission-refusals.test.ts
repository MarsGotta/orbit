import { expect, it } from "vitest";
import { receive } from "./interceptor";

it("nombra el permiso de calendario como Rust y abre su panel", () => {
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

it("nombra automatización y lleva a Ajustes sin pedir un panel que Tauri no admite", () => {
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
