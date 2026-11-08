import type { Permission } from "../../bridge/generated/Permission";
import type { PermissionState } from "../../bridge/generated";

/** Names and System Settings destinations of macOS permissions, shared by the UI. */
export const macPermissions = {
  microphone: { name: "microphone", label: "Microphone", pane: "Microphone" },
  screen_recording: {
    name: "screen recording",
    label: "Screen & System Audio",
    pane: "ScreenCapture",
  },
  accessibility: { name: "accessibility", label: "Accessibility", pane: "Accessibility" },
  screen_and_accessibility: {
    name: "screen recording and accessibility",
    pane: "ScreenCapture",
  },
  calendar: { name: "calendar", label: "Calendars", pane: "Calendars" },
  automation: { name: "automation" },
} as const satisfies Record<Permission, { name: string; label?: string; pane?: string }>;

export function macPermission(key: string) {
  return Object.hasOwn(macPermissions, key)
    ? macPermissions[key as keyof typeof macPermissions]
    : undefined;
}

export const permissionRows = [
  ["microphone", macPermissions.microphone],
  ["screen", macPermissions.screen_recording],
  ["accessibility", macPermissions.accessibility],
  ["calendars", macPermissions.calendar],
] as const;

export const permissions = {
  permissions: {
    granted: "Allowed",
    notGranted: "Not allowed",
  },
} as const;

/**
 * A macOS permission as the person reads it: granted or not. One
 * still to ask has its «Permitir» button beside it, and one not known yet says
 * nothing: an unknown is not a refusal.
 */
export function permissionStateLabel(state: PermissionState): string | null {
  if (state === "granted") return permissions.permissions.granted;
  if (state === "denied" || state === "not_granted") return permissions.permissions.notGranted;
  return null;
}
