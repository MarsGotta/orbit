import { people, type Person } from "./speaker-people";
import { useCopy } from "../../i18n/CopyProvider";
import { useRef, useState } from "react";
import type { NoteSpeakerSnapshot } from "../../bridge/generated/NoteSpeakerSnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { SpeakerRenameScope } from "../../bridge/generated/SpeakerRenameScope";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";
import { RadioList } from "../../design/RadioList";

import { formatTimestamp } from "./notes-format";
import styles from "./Note.module.css";

function Options({
  groups,
  typed,
  onAdd,
  onPick,
}: {
  groups: Array<[string, Person[]]>;
  typed: string;
  onAdd: () => void;
  onPick: (name: string) => void;
}) {
  const es = useCopy();
  return (
    <div className={styles.speakerOptions}>
      {groups.map(([label, list]) => (
        <div aria-label={label} key={label} role="group">
          <p className={styles.speakerGroup}>{label}</p>
          {list.map((person) => (
            <button
              className={styles.speakerOption}
              key={`${label}-${person.name}`}
              onClick={() => onPick(person.name)}
              type="button"
            >
              <span aria-hidden="true" className={styles.initial}>
                {person.name.charAt(0).toUpperCase()}
              </span>
              {person.name}
            </button>
          ))}
        </div>
      ))}
      {typed.length > 1 ? (
        <button className={styles.speakerAdd} onClick={() => onPick(typed)} type="button">
          {es.notes.addNamed(typed)}
        </button>
      ) : (
        <button className={styles.speakerAdd} onClick={onAdd} type="button">
          {es.notes.addSomeone}
        </button>
      )}
    </div>
  );
}

function Pick({
  note,
  speakerName,
  onPick,
}: {
  note: OpenNoteSnapshot;
  speakerName: string;
  onPick: (name: string) => void;
}) {
  const es = useCopy();
  const [query, setQuery] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const groups = people(note, query);
  const typed = query.trim();
  const first = groups[0]?.[1][0];
  return (
    <div>
      <div className={styles.speakerSearch}>
        <input
          aria-label={es.notes.speakerSearch}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            if (first) onPick(first.name);
            else if (typed.length > 1) onPick(typed);
          }}
          placeholder={es.notes.whoIs(speakerName)}
          ref={input}
          value={query}
        />
      </div>
      <Options groups={groups} onAdd={() => input.current?.focus()} onPick={onPick} typed={typed} />
    </div>
  );
}

/**
 * The last step: to which turns the name goes. A rename applies to this turn
 * (by its start) or to the voice, its future turns included.
 */
function Scope({
  name,
  note,
  speakerName,
  turn,
  onBack,
  onApply,
}: {
  name: string;
  note: OpenNoteSnapshot;
  speakerName: string;
  turn: NoteTurnSnapshot;
  onBack: () => void;
  onApply: (scope: SpeakerRenameScope, turns: number) => Promise<unknown>;
}) {
  const es = useCopy();
  const [scope, setScope] = useState<"turn" | "voice">("turn");
  const { feedback, track } = useFeedback();
  const turns = note.transcript.turns.filter(
    (other) => other.speaker_key === turn.speaker_key,
  ).length;
  return (
    <div className={styles.speakerScope}>
      <h2>{es.notes.wherePut(name)}</h2>
      <RadioList
        label={es.notes.wherePut(name)}
        name={`speaker-scope-${turn.index}`}
        onChange={setScope}
        options={[
          { value: "turn", label: es.notes.onlyThisTurn(formatTimestamp(turn.at_ms)) },
          { value: "voice", label: es.notes.allSpeakerTurns(turns, speakerName) },
        ]}
        value={scope}
      />
      <div className={styles.menuActions}>
        <Button onClick={onBack} size={32} variant="ghost">
          {es.notes.back}
        </Button>
        <Button
          feedback={feedback}
          onClick={() =>
            track(
              onApply(
                scope === "voice" ? { kind: "voice" } : { kind: "turn", at_ms: turn.at_ms },
                scope === "voice" ? turns : 1,
              ),
            )
          }
          pendingLabel={es.controls.saving}
          size={32}
          variant="accent"
        >
          {es.notes.applySpeaker}
        </Button>
      </div>
    </div>
  );
}

/**
 * «¿Quién es Persona 2?»: the meeting's guests and the voices already named,
 * a search that can add someone, and then where the name goes.
 */
export function SpeakerMenu({
  note,
  speaker,
  turn,
  onApply,
}: {
  note: OpenNoteSnapshot;
  speaker: NoteSpeakerSnapshot | undefined;
  turn: NoteTurnSnapshot;
  onApply: (name: string, scope: SpeakerRenameScope, turns: number) => Promise<unknown>;
}) {
  const [picked, setPicked] = useState<string | null>(null);
  const speakerName = speaker?.name ?? turn.speaker_key;
  return (
    <div className={styles.speakerMenu}>
      {picked === null ? (
        <Pick note={note} onPick={setPicked} speakerName={speakerName} />
      ) : (
        <Scope
          name={picked}
          note={note}
          onApply={(scope, turns) => onApply(picked, scope, turns)}
          onBack={() => setPicked(null)}
          speakerName={speakerName}
          turn={turn}
        />
      )}
    </div>
  );
}
