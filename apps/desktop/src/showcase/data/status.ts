import type { Agent } from "../../bridge/generated/Agent";
import type { Views } from "../scenario";
import { ZONE } from "./time";

/** The people of the invented company, Nubaria, and its client Hoteles Ribera. */
export const OWNER = "Lucía Ferrer";
export const OWNER_FIRST = "Lucía";

export const AGENTS: Agent[] = [
  { id: "deimos", name: "Deimos", voice: true },
  { id: "claude-code", name: "Claude Code", voice: false },
  { id: "codex", name: "Codex", voice: false },
];

export const status: Views["status"] = {
  availability: { state: "current" },
  connection: { daemon: "ready", hub: "paired" },
  privacy_stopped: false,
  system_timezone: ZONE,
  settings: {
    assistant_name: "Deimos",
    agents: AGENTS,
    default_agent_id: "deimos",
    attachments: {
      max_file_bytes: 20_971_520,
      max_message_files: 10,
      max_message_bytes: 33_554_432,
      max_session_bytes: 268_435_456,
    },
    hub: {
      url: "https://hub.example",
      paired: true,
      device_name: "Mac de Lucía",
      device_id_short: "7f3a2c",
      connection: "online",
    },
    helper_connected: true,
    calendars: { total: 4 },
    versions: { app: "0.10.0" },
    privacy_stopped: false,
    error_reports: true,
  },
  notice_availability: { state: "current" },
  notice_epoch: "showcase",
  // The log already holds a quiet notice: the window shows only what comes after it.
  notice: null,
  notices: [{ code: "store_recovered", op: "op-start", sequence: 1 }],
};

export const permissions: Views["permissions"] = {
  microphone: "granted",
  screen: "granted",
  accessibility: "granted",
  calendars: "granted",
  availability: { state: "current" },
};

export const local: Views["local"] = {
  profile: { name: OWNER_FIRST },
  login_item: "on",
  update: { state: "up_to_date", version: "0.10.0", installed_version: "0.10.0", message: null },
  onboarding_done: true,
};
