import { useEffect, useId, useState } from "react";
import type { DictionaryRequest } from "../../bridge/generated/DictionaryRequest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Dialog } from "../../design/Dialog";
import { Icon } from "../../design/Icon";
import { es } from "../../i18n/es";
import { sent, type Send } from "./send";
import styles from "./Note.module.css";

interface Props {
  wrong: string;
  note: OpenNoteSnapshot;
  library?: NoteLibrarySnapshot | null;
  send: Send;
  onClose: () => void;
  /** The hub has the correction: the transcript says so. */
  onSaved: (wrong: string, correct: string) => void;
}

/** The dictionary order; it rejects when the daemon refused it. */
function sendDictionary(send: Send, request: DictionaryRequest): Promise<void> {
  return sent(send, { command: "note_library", request: { action: "dictionary", request } });
}

function useCorrection({ wrong, note, library, send, onSaved }: Props) {
  const [correct, setCorrect] = useState(wrong);
  const [chosen, setChosen] = useState(note.dictionary?.dictionary_id ?? "");
  const [identity] = useState(() => crypto.randomUUID());
  const proposed = note.dictionary?.series?.title ?? "";
  const [name, setName] = useState(proposed.length > 0 ? proposed : note.title);
  const [submitted, setSubmitted] = useState(false);
  const id = chosen || identity;
  const dictionary = library?.dictionary_items?.find((item) => item.dictionary.id === id);
  const pending =
    dictionary?.pending === true ||
    (library?.dictionary_pending ?? []).includes(`dictionary:${id}`);
  const failures = (library?.dictionary_failures ?? []).filter(
    (failure) => failure.target === `dictionary:${id}`,
  );
  const saved =
    dictionary?.dictionary.terms.some(
      (entry) =>
        entry.correct === correct.trim() &&
        entry.wrong.some((form) => form.toLowerCase() === wrong.toLowerCase()),
    ) === true;
  useEffect(() => {
    if (submitted && !pending && saved) onSaved(wrong, correct.trim());
  }, [submitted, pending, saved, onSaved, wrong, correct]);
  const known = note.dictionary != null;
  const disabled = !known || !correct.trim() || (!chosen && !name.trim());
  const save = () => {
    if (disabled || pending) return;
    setSubmitted(true);
    // Refused: «Guardar corrección» can be pressed again; the notice says why.
    sendDictionary(send, {
      action: "correct",
      recording_id: note.recording_id,
      dictionary_id: id,
      wrong,
      correct: correct.trim(),
      name: chosen ? null : name.trim(),
    }).catch(() => setSubmitted(false));
  };
  const waiting = pending || (submitted && failures.length === 0 && !saved);
  return {
    correct,
    setCorrect,
    chosen,
    setChosen,
    name,
    setName,
    failures,
    known,
    disabled,
    waiting,
    save,
  };
}

/** A field of this window: its label above, as «Se oyó» and «Diccionario». */
function TextField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const id = useId();
  return (
    <div className={styles.dialogField}>
      <label htmlFor={id}>{label}</label>
      <input
        className={styles.dialogInput}
        id={id}
        onChange={(event) => onChange(event.currentTarget.value)}
        value={value}
      />
    </div>
  );
}

function DictionaryChoice({
  model,
  library,
  note,
}: {
  model: ReturnType<typeof useCorrection>;
  library: Props["library"];
  note: OpenNoteSnapshot;
}) {
  const id = useId();
  const items = library?.dictionary_items ?? [];
  const listed = items.some((item) => item.dictionary.id === model.chosen);
  return (
    <>
      <div className={styles.dialogField}>
        <label htmlFor={id}>{es.notes.dictionary}</label>
        <span className={styles.select}>
          <select
            disabled={library?.dictionaries?.state !== "available"}
            id={id}
            onChange={(event) => model.setChosen(event.currentTarget.value)}
            value={model.chosen}
          >
            {model.chosen !== "" && !listed && (
              <option value={model.chosen}>{note.dictionary?.series?.title ?? model.chosen}</option>
            )}
            {items.map(({ dictionary }) => (
              <option key={dictionary.id} value={dictionary.id}>
                {es.notes.dictionaryUsedBy(
                  dictionary.name,
                  dictionary.series.map((series) => series.title),
                )}
              </option>
            ))}
            <option value="">{es.notes.newDictionary}</option>
          </select>
          <Icon name="chevron" size={14} />
        </span>
      </div>
      {model.chosen === "" && (
        <TextField label={es.notes.dictionaryName} onChange={model.setName} value={model.name} />
      )}
    </>
  );
}

/**
 * «Corregir una palabra»: what was heard, what it should say, and the
 * dictionary that keeps the correction for every meeting that uses it.
 * The window stays, with what was typed, until the hub has the correction
 * or says why not.
 */
export function CorrectionDialog(props: Props) {
  const { wrong, note, library, onClose } = props;
  const model = useCorrection(props);
  return (
    <Dialog
      footer={
        <>
          <Button onClick={onClose} size={36} variant="ghost">
            {es.notes.cancel}
          </Button>
          <Button
            disabled={model.disabled}
            feedback={model.waiting ? "pending" : "idle"}
            onClick={model.save}
            pendingLabel={es.notes.savingCorrection}
            size={36}
            variant="accent"
          >
            {es.notes.saveCorrection}
          </Button>
        </>
      }
      onClose={onClose}
      subtitle={es.notes.correctSubtitle}
      title={es.notes.correctTitle}
    >
      <form
        className={styles.correction}
        onSubmit={(event) => {
          event.preventDefault();
          model.save();
        }}
      >
        <div className={styles.correctionPair}>
          <div className={styles.dialogField}>
            <span>{es.notes.heard}</span>
            <s className={styles.heard}>{wrong}</s>
          </div>
          <TextField label={es.notes.shouldSay} onChange={model.setCorrect} value={model.correct} />
        </div>
        <DictionaryChoice library={library} model={model} note={note} />
        <p className={styles.dialogNote}>{es.notes.correctLater}</p>
        {(!model.known || library?.dictionaries?.state !== "available") && (
          <p className={styles.dialogNote} role="status">
            {es.dictionary.unread}
          </p>
        )}
        {model.failures.map((failure, index) => (
          <p className={styles.menuRefusal} key={index} role="alert">
            {es.dictionary.refused(failure.code, failure.detail, failure.numbers)}
          </p>
        ))}
      </form>
    </Dialog>
  );
}
