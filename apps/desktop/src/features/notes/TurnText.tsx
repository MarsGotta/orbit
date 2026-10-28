import { useCopy } from "../../i18n/CopyProvider";
import { type ReactNode, useRef, useState } from "react";

import type { Match } from "./transcript-search";
import { type Span, wordAt } from "./word-forms";
import { offsetAtPoint, textOffset } from "./word-at-point";
import styles from "./Correction.module.css";

/** A word drawn apart: the one under the pointer, the one being corrected, or one just corrected. */
interface Marked extends Span {
  hover: boolean;
  picked: boolean;
  fixed: boolean;
}

function overlaps(one: Span, other: Span): boolean {
  return one.start < other.end && other.start < one.end;
}

/**
 * The words to draw apart, without overlaps: the one being corrected first,
 * then the corrected ones, then the one under the pointer; the same word in
 * two roles is drawn once with both.
 */
function marked(picked: Span | null, fixed: Span[], hover: Span | null): Marked[] {
  const kept: Marked[] = [];
  const add = (span: Span, role: keyof Omit<Marked, "start" | "end">) => {
    const same = kept.find((one) => one.start === span.start && one.end === span.end);
    if (same) same[role] = true;
    else if (!kept.some((one) => overlaps(one, span)))
      kept.push({ ...span, hover: false, picked: false, fixed: false, [role]: true });
  };
  if (picked) add(picked, "picked");
  for (const span of fixed) add(span, "fixed");
  if (hover) add(hover, "hover");
  return kept.sort((one, other) => one.start - other.start);
}

/** The search's marks within `[from, to)` of the text. */
function searched(
  text: string,
  from: number,
  to: number,
  matches: Match[],
  focused: Match | undefined,
): ReactNode[] {
  const parts: ReactNode[] = [];
  let at = from;
  for (const match of matches) {
    const start = Math.max(match.start, from);
    const end = Math.min(match.end, to);
    if (end <= start) continue;
    if (start > at) parts.push(text.slice(at, start));
    parts.push(
      <mark data-current={match === focused} key={`m${start}`}>
        {text.slice(start, end)}
      </mark>,
    );
    at = end;
  }
  if (at < to) parts.push(text.slice(at, to));
  return parts;
}

export interface TurnTextProps {
  /** What the turn says, a correction already in. */
  text: string;
  matches: Match[];
  focused: Match | undefined;
  /** The words just corrected, in `--o-okSoft` until the note is left. */
  fixed: Span[];
  /** The word whose menu is open, in `--o-sel`. */
  picked: Span | null;
  /** The word was pressed: its place in the text. */
  onPick: (span: Span) => void;
  /** The element of the word being corrected, for its menu to hang from. */
  anchorRef: (element: HTMLElement | null) => void;
}

/** The word a press landed on: where the press put the caret, a collapsed selection. */
function pressed(paragraph: HTMLElement, text: string): Span | null {
  const selection = window.getSelection();
  if (selection === null || !selection.isCollapsed || selection.anchorNode === null) return null;
  const offset = textOffset(paragraph, selection.anchorNode, selection.anchorOffset);
  if (offset === null) return null;
  return wordAt(text, offset) ?? wordAt(text, offset - 1);
}

/** Whether the user is selecting text in it: a press then is a selection, not a word. */
function selecting(paragraph: HTMLElement): boolean {
  const selection = window.getSelection();
  return (
    selection !== null &&
    !selection.isCollapsed &&
    selection.anchorNode !== null &&
    paragraph.contains(selection.anchorNode)
  );
}

/**
 * What a turn said (annex, change 5): every word answers the pointer with
 * `--o-wordHover` and «Pulsa para corregir», and a press opens its menu. The
 * words are not in the keyboard's order; with the keyboard, the text is
 * selected and «Corregir «…»» opens the same menu.
 */
export function TurnText(props: TurnTextProps) {
  const es = useCopy();
  const { text, matches, focused, fixed, picked, onPick, anchorRef } = props;
  const paragraph = useRef<HTMLParagraphElement>(null);
  const [hover, setHover] = useState<Span | null>(null);
  const point = (x: number, y: number) => {
    const at = paragraph.current && offsetAtPoint(paragraph.current, x, y);
    return at == null ? null : wordAt(text, at);
  };
  const pieces: ReactNode[] = [];
  let at = 0;
  for (const word of marked(picked, fixed, hover)) {
    pieces.push(...searched(text, at, word.start, matches, focused));
    pieces.push(
      <span
        className={styles.word}
        data-fixed={word.fixed ? "" : undefined}
        data-hover={word.hover ? "" : undefined}
        data-picked={word.picked ? "" : undefined}
        key={`w${word.start}`}
        ref={word.picked ? anchorRef : undefined}
      >
        {searched(text, word.start, word.end, matches, focused)}
        {word.hover && !word.picked && (
          <span aria-hidden="true" className={styles.wordTip}>
            {es.notes.pressToCorrect}
          </span>
        )}
      </span>,
    );
    at = word.end;
  }
  pieces.push(...searched(text, at, text.length, matches, focused));
  return (
    <p
      onClick={() => {
        if (!paragraph.current || selecting(paragraph.current)) return;
        const word = hover ?? pressed(paragraph.current, text);
        if (word) onPick(word);
      }}
      onMouseLeave={() => setHover(null)}
      onMouseMove={(event) => {
        // A drag selects text; drawing a word apart then would break the selection.
        if (event.buttons !== 0 || !paragraph.current || selecting(paragraph.current)) return;
        const word = point(event.clientX, event.clientY);
        if (word?.start !== hover?.start || word?.end !== hover?.end) setHover(word);
      }}
      ref={paragraph}
    >
      {pieces}
    </p>
  );
}
