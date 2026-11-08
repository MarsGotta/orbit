import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Icon } from "../../design/Icon";
import { EditMenu } from "../../design/EditMenu";
import { Field } from "../../design/Field";
import { MultiSelect } from "../../design/MultiSelect";

import {
  chosen,
  correctionOrder,
  dictionaryOptions,
  holds,
  meetingDictionaries,
  nameOf,
  orderTarget,
  pending,
  refusalOf,
  takesLists,
} from "./dictionary-choice";
import { sent, type Send } from "./send";
import { occurrences } from "./word-forms";
import styles from "./Correction.module.css";

/** A correction the hub took: what it changed and where it keeps it. */
export interface Correction {
  wrong: string;
  correct: string;
  /** The dictionaries it went into, in the order sent: «Deshacer» takes it out of them all. */
  ids: string[];
  names: string[];
  /** The places of the meeting it changed. */
  places: number;
  /** The turns as they read when it was asked for, before the hub's read brings it. */
  before: NoteTurnSnapshot[];
  /** What the daemon files the order's answers under. */
  target: string;
}

export interface WordMenuProps {
  /** The word pressed, or the text selected. */
  wrong: string;
  /** What it hangs from: the word, or «Corregir «…»» under a selection. */
  anchor: HTMLElement | null;
  note: OpenNoteSnapshot;
  library?: NoteLibrarySnapshot | null;
  send: Send;
  onClose: () => void;
  onSaved: (correction: Correction) => void;
}

/** The places of the meeting that say `wrong`, as the hub's dictionaries find them. */
function timesIn(note: OpenNoteSnapshot, wrong: string): number {
  return note.transcript.turns.reduce((sum, turn) => sum + occurrences(turn.text, wrong).length, 0);
}

/** The choice of dictionaries: the meeting's own, or a new one when there is none yet. */
function useDictionaries(note: OpenNoteSnapshot, library: WordMenuProps["library"]) {
  const meeting = meetingDictionaries(note) ?? [];
  const none = (library?.dictionary_items ?? []).length === 0;
  const [ids, setIds] = useState(meeting);
  const [creating, setCreating] = useState(none && meeting.length === 0);
  const [newId] = useState(() => crypto.randomUUID());
  const proposed = note.dictionary?.series?.title ?? "";
  const [name, setName] = useState(proposed.length > 0 ? proposed : note.title);
  const all = creating ? [...ids, newId] : ids;
  const names = [...ids.map((id) => nameOf(library, id)), ...(creating ? [name.trim()] : [])];
  const choose = (next: string[]) => {
    const kept = chosen(note, ids, next);
    setIds(kept);
    // An older hub takes one dictionary: choosing one drops the new one.
    if (!takesLists(note) && kept.length > 0) setCreating(false);
  };
  const create = () => {
    setCreating(true);
    if (!takesLists(note)) setIds([]);
  };
  return { meeting, ids, all, names, creating, name, setName, choose, create };
}

type Choice = ReturnType<typeof useDictionaries>;

/** The order from the press until the hub's read shows it saved, or says why not. */
function useCorrection(props: WordMenuProps, correct: string, choice: Choice) {
  const { wrong, note, library, send, onSaved } = props;
  const [submitted, setSubmitted] = useState<{
    target: string;
    before: NoteTurnSnapshot[];
    times: number;
  } | null>(null);
  const times = timesIn(note, wrong);
  const target = submitted?.target ?? "";
  const onItsWay = submitted !== null && pending(library, target);
  const refused = submitted === null || onItsWay ? undefined : refusalOf(library, target);
  const saved =
    submitted !== null &&
    !onItsWay &&
    refused === undefined &&
    holds(library, choice.all, wrong, correct);
  // Once, when the hub's read shows it saved.
  const told = useRef(false);
  useEffect(() => {
    if (!saved || told.current || submitted === null) return;
    told.current = true;
    const { before } = submitted;
    onSaved({
      wrong,
      correct,
      ids: choice.all,
      names: choice.names,
      places: submitted.times,
      before,
      target,
    });
  });
  const save = () => {
    const name = choice.creating ? choice.name.trim() : null;
    const order = correctionOrder(note, choice.all, wrong, correct, name);
    setSubmitted({ target: orderTarget(order), before: note.transcript.turns, times });
    // Refused at once: «Corregir y añadir» can be pressed again; the notice says why.
    sent(send, {
      command: "note_library",
      request: { action: "dictionary", request: order },
    }).catch(() => setSubmitted(null));
  };
  return { times, refused, waiting: submitted !== null && refused === undefined && !saved, save };
}

/**
 * «Corregir «Orvit»» (annex, change 5): what it should say, how many times
 * it is said in this meeting, and the dictionaries it goes into, several at
 * once. The menu stays, with what was typed, until the hub has the
 * correction or says why not.
 */
export function WordMenu(props: WordMenuProps) {
  const es = useCopy();
  const { wrong, anchor, note, library, onClose } = props;
  const [correct, setCorrect] = useState(wrong);
  const choice = useDictionaries(note, library);
  const right = correct.trim();
  const { times, refused, waiting, save } = useCorrection(props, right, choice);
  const readable = note.dictionary !== undefined && library?.dictionaries?.state === "available";
  const valid =
    readable &&
    times > 0 &&
    right !== "" &&
    right !== wrong &&
    choice.all.length > 0 &&
    (!choice.creating || choice.name.trim() !== "");
  const error = !readable ? es.dictionary.unread : times === 0 ? es.notes.notWhole(wrong) : refused;
  return (
    <EditMenu
      // In the lower half of the window it opens upwards, so it never covers the lines below.
      above={anchor !== null && anchor.getBoundingClientRect().top > window.innerHeight / 2}
      anchor={anchor}
      canSave={valid && !waiting}
      error={error}
      feedback={waiting ? "pending" : "idle"}
      onClose={onClose}
      onSave={save}
      saveLabel={es.notes.correctAndAdd}
      subtitle={times > 0 ? es.notes.timesHere(times) : undefined}
      title={es.notes.correct(wrong)}
      width={420}
    >
      <Field
        label={es.notes.shouldSay}
        onChange={(event) => setCorrect(event.target.value)}
        value={correct}
      />
      {times > 1 && <EveryPlace times={times} />}
      <DictionaryChoice choice={choice} library={library} />
    </EditMenu>
  );
}

/**
 * «Corregir las 3 veces», ticked for good: a dictionary corrects every place a
 * form is said, and the hub has no correction for one place alone.
 */
function EveryPlace({ times }: { times: number }) {
  const es = useCopy();
  return (
    <span aria-checked="true" aria-disabled="true" className={styles.every} role="checkbox">
      <span aria-hidden="true" className={styles.everyBox}>
        <Icon name="check" size={14} />
      </span>
      {es.notes.correctEvery(times)}
    </span>
  );
}

/** «Se añade a · puedes elegir varios», a new dictionary's name, and where it will apply. */
function DictionaryChoice({
  choice,
  library,
}: {
  choice: Choice;
  library: WordMenuProps["library"];
}) {
  const es = useCopy();
  return (
    <>
      <MultiSelect
        createLabel={es.notes.newDictionary}
        label={es.notes.addTo}
        onChange={choice.choose}
        onCreate={choice.create}
        options={dictionaryOptions(library, choice.meeting)}
        placeholder={es.notes.chooseDictionaries}
        selected={choice.ids}
      />
      {choice.creating && (
        <Field
          label={es.notes.dictionaryName}
          onChange={(event) => choice.setName(event.target.value)}
          value={choice.name}
        />
      )}
      <p className={styles.where}>
        {choice.all.length > 0 ? es.notes.appliesIn(choice.names) : es.notes.chooseOne}
      </p>
    </>
  );
}
