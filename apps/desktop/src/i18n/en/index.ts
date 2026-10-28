import { interfaceCopy } from "./interface";
import { defineCatalogue } from "../catalogue";
import { assistantsCopy, vendorOf, shortDay } from "./assistants";
import { accessGroups, agentFailureText, agentRefusalText } from "./agents";
import { permissionRows, macPermissions } from "./permissions";
import { fileSize, attachmentRefusalText } from "./attachments";
import { providerRefusedAudio } from "./voice-failures";
import { aiCopy, aiRefusalText } from "./ai";
import { usageWindow } from "./assistantsFormat";
import { transcription } from "./transcription";
import { vaultCopy } from "./vault";
import { waits } from "./waits";
import { refusalEntries } from "./refusalEntries";
import { noticesForAssistant } from "./notices";
import { agents } from "./agents";
import { dictionary } from "./dictionary";
import { statusCopy } from "./status";
import { shellCopy } from "./shell";
import { controls } from "./controls";
import { permissions } from "./permissions";
import { briefingCopy } from "./briefing";
import { settingsCopy } from "./settings";
import { recording } from "./recording";
import { agenda } from "./agenda";
import { notes } from "./notes";
import { assistantCopy } from "./assistant";
import { chatActionsCopy } from "./chatActions";
import { attachmentsCopy } from "./attachments";
import { toolFailureText } from "./notices/toolFailure";
import { vaultWarningText } from "./vaultWarnings";
import { vaultRefusalText } from "./vaultRefusals";
import { errors } from "./errors";

import { noteScreenCopy } from "./noteScreen";
import { library } from "./library";
import { meetingQuestionCopy } from "./meetingQuestion";
import { floatingPanelCopy } from "./floatingPanel";
import { onboardingCopy } from "./onboarding";
import { paletteCopy } from "./palette";
import { failures } from "./failures";
import { editorCopy } from "./editor";

import { globalShortcuts } from "./shortcuts";
export { globalShortcuts };

export function english(assistantName = "Deimos") {
  return defineCatalogue({
    ...interfaceCopy,
    extras: {
      assistantsCopy,
      vendorOf,
      shortDay,
      accessGroups,
      agentFailureText,
      agentRefusalText,
      permissionRows,
      macPermissions,
      fileSize,
      attachmentRefusalText,
      providerRefusedAudio,
      aiCopy,
      aiRefusalText,
      usageWindow,
      transcription,
      vaultCopy,
      waits,
      refusalEntries,
      noticesForAssistant,
      globalShortcuts,
      failures,
      vaultRefusalText,
      vaultWarningText,
      toolFailureText,
    },
    ...agents,
    ...dictionary,
    ...statusCopy(assistantName),
    ...shellCopy(assistantName),
    ...controls,
    ...permissions,
    ...briefingCopy(assistantName),
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
    ...onboardingCopy(assistantName),
    ...paletteCopy(assistantName),
    ...failures,
    ...editorCopy,
  } as const);
}

export const en = english();
