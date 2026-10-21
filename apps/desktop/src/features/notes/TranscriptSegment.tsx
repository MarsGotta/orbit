import { type CSSProperties, type ReactNode, forwardRef } from "react";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { formatTimestamp, speakerName, stableColor } from "./notes-format";
import type { Match } from "./transcript-search";
import styles from "./NoteScreen.module.css";

function highlighted(text: string, matches: Match[], focused: Match | undefined): ReactNode {
  if (matches.length === 0) return text;
  const parts: ReactNode[] = [];
  let at = 0;
  for (const match of matches) {
    if (match.start > at) parts.push(text.slice(at, match.start));
    parts.push(
      <mark data-current={match === focused} key={match.start}>
        {text.slice(match.start, match.end)}
      </mark>,
    );
    at = match.end;
  }
  if (at < text.length) parts.push(text.slice(at));
  return parts;
}

export interface SegmentProps {
  focused: Match | undefined;
  matches: Match[];
  note: OpenNoteSnapshot;
  onSpeaker: (turn: NoteTurnSnapshot) => void;
  turn: NoteTurnSnapshot;
}

/** One turn: its time from the start, who speaks (in a colour that stays
 * theirs) and what was said, as the hub computed them. */
export const TranscriptSegment = forwardRef<HTMLElement, SegmentProps>(function TranscriptSegment(
  { focused, matches, note, onSpeaker, turn },
  ref,
) {
  const label = speakerName(note, turn);
  const color =
    note.speakers.find((speaker) => speaker.key === turn.speaker_key)?.color ?? stableColor(label);
  return (
    <article
      className={styles.segment}
      ref={ref}
      style={{ "--speaker-color": color } as CSSProperties}
    >
      <header>
        <time>{formatTimestamp(turn.at_ms)}</time>
        <span aria-hidden="true" className={styles.speakerDot}>
          {label.charAt(0).toUpperCase()}
        </span>
        <button className={styles.speakerName} onClick={() => onSpeaker(turn)} type="button">
          {label}
        </button>
      </header>
      <p>{highlighted(turn.text, matches, focused)}</p>
    </article>
  );
});
