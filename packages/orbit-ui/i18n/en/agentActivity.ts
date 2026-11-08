import type { ActivityCopy } from "../../bridge/generated/ActivityCopy";

import type { ActivityLabelCode } from "../../bridge/generated/ActivityLabelCode";

const labels: Record<Exclude<ActivityLabelCode, "unknown">, string> = {
  click_app: "Clicking in {value}",
  click: "Clicking on the screen",
  type: "Typing",
  keys: "Pressing keys",
  scroll_app: "Scrolling through {value}",
  scroll: "Scrolling",
  open_app: "Opening {value}",
  open: "Opening an app",
  action_tool: "Using {value} on the Mac",
  action: "Acting on the Mac",
  command_value: "Running: {value}",
  command: "Running a command",
  edit_value: "Editing: {value}",
  edit: "Editing files",
  screenshot_app: "Looking at {value}",
  screenshot: "Looking at the screen",
  plan: "Plan · {done} of {total} steps",
  orbit_devices: "Looking at the Orbit Macs",
  orbit_windows: "Looking at the windows",
  orbit_look: "Looking at the screen",
  orbit_selected_text: "Reading the selected text",
  orbit_control_tools: "Checking what it can do on the Mac",
  orbit_action_result: "Checking how the action ended",
  orbit_calendar: "Reading the calendar",
  orbit_notes_search: "Searching the notes",
  orbit_note_read: "Reading a note",
  orbit_notes_index: "Reading the notes index",
  orbit_meetings: "Reading the meetings",
  orbit_transcript: "Reading a meeting transcript",
  orbit_note: "Reading a meeting note",
  orbit_files_list: "Looking at the Mac folders",
  orbit_files_search: "Searching the Mac folders",
  orbit_files_read: "Reading a Mac file",
  orbit_attachment: "Reading a message attachment",
  orbit_skill: "Loading one of its skills",
};

const subjects = {
  hub_terminal: "the hub terminal",
  hub_files: "hub files",
  macs: "the Macs",
  windows: "the windows",
  screen: "the screen",
  selected_text: "the selected text",
  mac: "the Mac",
  calendar: "the calendar",
  notes: "the notes",
  note: "a note",
  notes_index: "the notes index",
  meetings: "the meetings",
  folders: "the folders",
  attachments: "the attachments",
  skills: "its skills",
};

export const agentActivity = {
  labels,
  subjects,
  label(copy: ActivityCopy | undefined, fallback: string): string {
    if (!copy || !Object.hasOwn(labels, copy.code)) return fallback;
    const values = {
      value: copy.value ?? "",
      done: String(copy.done ?? 0),
      total: String(copy.total ?? 0),
    };
    return labels[copy.code as keyof typeof labels].replace(
      /\{(value|done|total)\}/g,
      (_, key: keyof typeof values) => values[key],
    );
  },
  subject(code: string | undefined, fallback: string): string {
    return code && Object.hasOwn(subjects, code)
      ? subjects[code as keyof typeof subjects]
      : fallback;
  },
};
