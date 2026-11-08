import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import { AttentionRow } from "../../design/AttentionRow";
import { Button } from "../../design/Button";

import type { StartBlocker as Blocker } from "./start-options";

/** What stops the recording, once, with the button that fixes it. */
export function StartBlocker({
  blocker,
  send,
}: {
  blocker: Blocker;
  send: (command: AppCommand) => void;
}) {
  const es = useCopy();
  const macPermissions = useCopy().extras.macPermissions;
  if (blocker === "privacy_stopped") {
    return (
      <AttentionRow
        action={
          <Button onClick={() => send({ command: "rearm" })} size={28}>
            {es.meeting.rearm}
          </Button>
        }
        help={es.meeting.privacyStoppedHelp}
        title={es.meeting.privacyStopped}
      />
    );
  }
  const microphone = blocker === "microphone";
  const pane = microphone ? macPermissions.microphone.pane : macPermissions.screen_recording.pane;
  return (
    <AttentionRow
      action={
        <Button onClick={() => send({ command: "open_system_privacy", pane })} size={28}>
          {es.meeting.openSystemSettings}
        </Button>
      }
      help={microphone ? es.meeting.missingMicrophoneHelp : es.meeting.missingScreenHelp}
      title={microphone ? es.meeting.missingMicrophone : es.meeting.missingScreen}
    />
  );
}
