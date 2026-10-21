import type { AppCommand } from "../bridge/generated";
import type { CommandItem } from "../design/CommandPalette";
import { es } from "../i18n/es";
import type { ErrorOrigin } from "../state/orbit-store";
import type { ViewId } from "../state/ui";

export function paletteCommands(
  navigate: (view: ViewId) => void,
  send: (command: AppCommand, origin?: ErrorOrigin) => void,
  openMeeting: () => void,
): CommandItem[] {
  return [
    {
      id: "call",
      icon: "phone",
      label: es.design.callOrbit,
      subtitle: es.design.callSubtitle,
      onSelect: () => {
        navigate("assistant");
        // The call's refusal belongs to Assistant, where the palette leaves
        // the user, not to the view they opened the palette from.
        send({ command: "call_start", duration_minutes: 60 }, "assistant");
      },
    },
    {
      id: "voice",
      icon: "mic",
      label: es.assistant.paletteVoice,
      subtitle: es.assistant.paletteVoiceSubtitle,
      onSelect: () => navigate("assistant"),
    },
    {
      id: "briefing",
      icon: "briefing",
      label: es.assistant.paletteBriefing,
      subtitle: es.assistant.paletteBriefingSubtitle,
      onSelect: () => navigate("briefing"),
    },
    {
      id: "record-meeting",
      icon: "mic",
      label: es.meeting.recordNow,
      subtitle: es.meeting.recordNowSubtitle,
      onSelect: openMeeting,
    },
  ];
}
