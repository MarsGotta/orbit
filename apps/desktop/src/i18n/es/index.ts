import { agents } from "./agents";
import { dictionary } from "./dictionary";
import { status } from "./status";
import { permissions } from "./permissions";
import { briefing } from "./briefing";
import { settingsCopy } from "./settings";
import { recording } from "./recording";
import { agenda } from "./agenda";
import { notes } from "./notes";
import { assistantCopy } from "./assistant";
import { chatActions } from "./chatActions";
import { errors } from "./errors";

import { noteScreenCopy } from "./noteScreen";
import { library } from "./library";
import { meetingQuestionCopy } from "./meetingQuestion";

import { globalShortcuts } from "./shortcuts";
export { globalShortcuts };

export function spanish(assistantName = "Deimos") {
  return {
    ...agents,
    ...dictionary,
    ...status,
    ...permissions,
    ...briefing,
    ...settingsCopy(assistantName),
    ...recording,
    ...agenda,
    ...notes,
    ...assistantCopy(assistantName),
    ...chatActions,
    ...errors,
    ...noteScreenCopy(assistantName),
    ...library,
    ...meetingQuestionCopy(assistantName),
  } as const;
}

export const es = spanish();
