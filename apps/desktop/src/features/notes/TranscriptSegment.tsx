import { type CSSProperties, type ReactNode, forwardRef } from "react";
import type { NoteSpeakerSnapshot } from "../../bridge/generated/NoteSpeakerSnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import { es } from "../../i18n/es";
import { formatTimestamp, isUnnamedSpeaker } from "./notes-format";
import type { Match } from "./transcript-search";
import styles from "./Note.module.css";

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
  turn: NoteTurnSnapshot;
  /** Who speaks: the button that opens «¿Quién es…?», or that menu open. */
  who: ReactNode;
  /** Under the text: «Corregir «…»» for a selected word. */
  after?: ReactNode;
}

/** The speaker's name in the turn: their colour, and «· ponle nombre» when nobody named them. */
export function SpeakerName({
  speaker,
  turn,
}: {
  speaker: NoteSpeakerSnapshot | undefined;
  turn: NoteTurnSnapshot;
}) {
  const unnamed = isUnnamedSpeaker(speaker);
  return (
    <>
      <span
        aria-hidden="true"
        className={styles.speakerDot}
        style={{ background: unnamed ? "var(--o-ink3)" : speaker?.color } as CSSProperties}
      />
      {speaker?.name ?? turn.speaker_key}
      {unnamed && <span className={styles.nameIt}>{` ${es.notes.nameIt}`}</span>}
    </>
  );
}

/** One turn: its time from the start, who speaks and what was said, as the hub computed them. */
export const TranscriptSegment = forwardRef<HTMLElement, SegmentProps>(function TranscriptSegment(
  { focused, matches, turn, who, after },
  ref,
) {
  return (
    <article className={styles.turn} data-turn={turn.index} ref={ref}>
      <time>{formatTimestamp(turn.at_ms)}</time>
      <div className={styles.turnBody}>
        {who}
        <p>{highlighted(turn.text, matches, focused)}</p>
        {after}
      </div>
    </article>
  );
});
