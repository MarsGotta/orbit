import type { AppCommand } from "../../bridge/generated";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { useSpanish } from "../../i18n/AssistantCopy";
import { AskDeimos } from "./AskDeimos";
import { voiceStatusShort, voiceStatusText } from "./copy";
import styles from "./MeetingQuestion.module.css";

/**
 * P9-12: «Preguntar a Deimos» under the recording pill. It stays while Mars
 * writes, whatever the pointer does, and only while app-core offers it.
 */
export function PillAsk({
  meeting,
  onDone,
  send,
}: {
  meeting: MeetingSnapshot;
  onDone: () => void;
  send: (command: AppCommand) => void;
}) {
  if (!meeting.ask) return null;
  return (
    <div className={styles.popover}>
      <AskDeimos
        ask={meeting.ask}
        onDone={onDone}
        send={send}
        voiceStatus={meeting.voice_trigger}
      />
    </div>
  );
}

/** «Preguntar a Deimos» beside pause and finish, while app-core offers it. */
export function PillAskButton({
  className,
  meeting,
  onAsk,
}: {
  className?: string;
  meeting: MeetingSnapshot;
  onAsk: () => void;
}) {
  const es = useSpanish();
  if (!meeting.ask) return null;
  return (
    <button className={className} onClick={onAsk}>
      {es.meetingQuestion.ask}
    </button>
  );
}

/**
 * P9-12: in the pill, whether «Oye Deimos» listens during the recording;
 * the full sentence, with its cause, is its title and its accessible name.
 */
export function PillVoiceStatus({
  className,
  meeting,
}: {
  className?: string;
  meeting: MeetingSnapshot;
}) {
  const es = useSpanish();
  const status = meeting.voice_trigger;
  const short = status ? voiceStatusShort(status, es) : null;
  if (!status || short === null) return null;
  const full = voiceStatusText(status, es) ?? undefined;
  return (
    <span aria-label={full} className={className} title={full}>
      {short}
    </span>
  );
}
