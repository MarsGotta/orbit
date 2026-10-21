import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { useModalFocus } from "../../design/use-modal-focus";
import { es } from "../../i18n/es";
import { DictionaryFeedback, sendDictionary } from "./DictionaryPicker";
import styles from "./SpeakerRenameDialog.module.css";

interface Props {
  wrong: string;
  note: OpenNoteSnapshot;
  library?: NoteLibrarySnapshot | null;
  send: (command: AppCommand) => void;
  onClose: () => void;
}

function useCorrection({ wrong, note, library, send, onClose }: Props) {
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
  const saved =
    dictionary?.dictionary.terms.some(
      (entry) =>
        entry.correct === correct.trim() &&
        entry.wrong.some((form) => form.toLowerCase() === wrong.toLowerCase()),
    ) === true;
  useEffect(() => {
    if (submitted && !pending && saved) onClose();
  }, [submitted, pending, saved, onClose]);
  const known = note.dictionary != null;
  const disabled = !known || pending || !correct.trim() || (!chosen && !name.trim());
  const save = () => {
    if (disabled) return;
    setSubmitted(true);
    sendDictionary(send, {
      action: "correct",
      recording_id: note.recording_id,
      dictionary_id: id,
      wrong,
      correct: correct.trim(),
      name: chosen ? null : name.trim(),
    });
  };
  return { correct, setCorrect, chosen, setChosen, name, setName, pending, known, disabled, save };
}

function DictionaryChoice({
  model,
  library,
}: {
  model: ReturnType<typeof useCorrection>;
  library: Props["library"];
}) {
  return (
    <>
      <label className={styles.name}>
        {es.dictionary.label}
        <select
          value={model.chosen}
          disabled={library?.dictionaries?.state !== "available"}
          onChange={(event) => model.setChosen(event.currentTarget.value)}
        >
          <option value="">{es.dictionary.create}</option>
          {(library?.dictionary_items ?? []).map(({ dictionary }) => (
            <option key={dictionary.id} value={dictionary.id}>
              {dictionary.name}
            </option>
          ))}
        </select>
      </label>
      {!model.chosen && (
        <label className={styles.name}>
          {es.dictionary.name}
          <input
            value={model.name}
            onChange={(event) => model.setName(event.currentTarget.value)}
          />
        </label>
      )}
    </>
  );
}

export function CorrectionDialog(props: Props) {
  const { wrong, note, library, onClose } = props;
  const model = useCorrection(props);
  const panel = useModalFocus(true);
  return (
    <div className={styles.backdrop} role="presentation">
      <section
        className={styles.dialog}
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={es.dictionary.correctText}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            onClose();
          }
        }}
      >
        <h2>{es.dictionary.correctText}</h2>
        <p className={styles.dictionaryText}>{es.dictionary.selected(wrong)}</p>
        <form
          className={styles.dictionaryForm}
          onSubmit={(event) => {
            event.preventDefault();
            model.save();
          }}
        >
          <label className={styles.name}>
            {es.dictionary.correctForm}
            <input
              value={model.correct}
              onChange={(event) => model.setCorrect(event.currentTarget.value)}
            />
          </label>
          {note.dictionary?.dictionary_id == null && (
            <DictionaryChoice model={model} library={library} />
          )}
          <p>{es.dictionary.help}</p>
          {!model.known && <p role="status">{es.dictionary.unread}</p>}
          <DictionaryFeedback library={library} />
          {model.pending && <p role="status">{es.dictionary.saving}</p>}
          <footer>
            <Button onClick={onClose}>{es.notes.cancel}</Button>
            <Button type="submit" variant="accent" disabled={model.disabled}>
              {es.dictionary.saveCorrection}
            </Button>
          </footer>
        </form>
      </section>
    </div>
  );
}
