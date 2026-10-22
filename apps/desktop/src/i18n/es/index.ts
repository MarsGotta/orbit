import { agents } from "./agents";
import { dictionary } from "./dictionary";
import { statusCopy } from "./status";
import { controls } from "./controls";
import { permissions } from "./permissions";
import { briefing } from "./briefing";
import { settingsCopy } from "./settings";
import { recording } from "./recording";
import { agenda } from "./agenda";
import { notes } from "./notes";
import { assistantCopy } from "./assistant";
import { chatActionsCopy } from "./chatActions";
import { attachmentsCopy } from "./attachments";
import { errors } from "./errors";

import { noteScreenCopy } from "./noteScreen";
import { library } from "./library";
import { meetingQuestionCopy } from "./meetingQuestion";
import { floatingPanelCopy } from "./floatingPanel";

import { globalShortcuts } from "./shortcuts";
export { globalShortcuts };

export function spanish(assistantName = "Deimos") {
  return {
    ...agents,
    ...dictionary,
    ...statusCopy(assistantName),
    ...controls,
    ...permissions,
    ...briefing,
    ...settingsCopy(assistantName),
    ...recording,
    ...agenda,
    ...notes,
    ...assistantCopy(assistantName),
    ...chatActionsCopy(assistantName),
    ...attachmentsCopy,
    ...errors,
    ...noteScreenCopy(assistantName),
    ...library,
    ...meetingQuestionCopy(assistantName),
    ...floatingPanelCopy(assistantName),
  } as const;
}

export const es = spanish();
