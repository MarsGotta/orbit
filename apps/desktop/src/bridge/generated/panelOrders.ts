// Generated from PanelOrder by orbit-app-core's command vocabulary test. Do not edit.
import type { PanelOrder } from "./PanelOrder";
export const panelOrders = {
  "call_dismiss": true,
  "call_record": true,
  "meeting_ask": true,
  "meeting_finish": true,
  "meeting_question_close": true,
  "open_url": true,
  "panel_focus": true,
  "recording_keep": true
} as const satisfies Record<PanelOrder["command"], true>;
