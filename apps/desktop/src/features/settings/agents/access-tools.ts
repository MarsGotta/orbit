import type { AccessGroups } from "../../../bridge/generated/AccessGroups";

/**
 * The orbit-mcp tools each access group opens, shown beside its switch. The
 * hub decides what each group opens (`TOOLS` in
 * `server/orbit_hub/assistant/access.py`); this is its copy for the screen.
 * `contracts/constants.json` registers both (AGENT_ACCESS_GROUP_TOOLS) and
 * `test_constants_copies.py` compares them. `orbit_devices` and
 * `orbit_skill` belong to no group.
 */
export const ACCESS_GROUP_TOOLS = {
  screen: ["orbit_windows", "orbit_screenshot", "orbit_look", "orbit_selected_text"],
  control: ["orbit_control_tools", "orbit_perform_action", "orbit_action_result"],
  calendar: ["orbit_calendar"],
  meetings: ["orbit_meetings", "orbit_transcript", "orbit_note"],
  files: ["orbit_files_list", "orbit_files_search", "orbit_files_read"],
} as const satisfies Record<keyof AccessGroups, readonly string[]>;
