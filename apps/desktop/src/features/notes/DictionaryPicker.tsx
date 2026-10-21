import type { AppCommand } from "../../bridge/generated";
import type { DictionaryRequest } from "../../bridge/generated/DictionaryRequest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import styles from "./NoteScreen.module.css";
import { es } from "../../i18n/es";

type Send = (command: AppCommand) => void;

export function sendDictionary(send: Send, request: DictionaryRequest) {
  send({ command: "note_library", request: { action: "dictionary", request } });
}

export function DictionaryFeedback({ library }: { library?: NoteLibrarySnapshot | null }) {
  const state = library?.dictionaries;
  return (
    <>
      {state?.state !== "available" && (
        <p role={state == null || state.state === "unknown" ? undefined : "status"}>
          {state?.detail ?? es.dictionary.unread}
        </p>
      )}
      {library?.read === "failed" && <p role="alert">{library.read_detail}</p>}
      {(library?.dictionary_failures ?? []).map((failure, index) => (
        <p role="alert" key={index}>
          {es.dictionary.refused(failure.code, failure.detail, failure.numbers)}
        </p>
      ))}
    </>
  );
}

export function DictionaryPicker({
  library,
  note,
  send,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  send: Send;
}) {
  const current = note.dictionary;
  const available = library?.dictionaries?.state === "available";
  const pending = (library?.dictionary_pending ?? []).includes(
    `dictionary_meeting:${note.recording_id}`,
  );
  const selected = current?.dictionary_id ?? "";
  const choose = (dictionary_id: string | null, use_for_series: boolean) =>
    sendDictionary(send, {
      action: "select",
      recording_id: note.recording_id,
      dictionary_id,
      use_for_series,
    });
  return (
    <div>
      <label className={`${styles.templatePicker} ${styles.dictionaryPicker}`}>
        <span>{es.dictionary.label}</span>
        <select
          disabled={!available || current == null || pending}
          value={selected}
          onChange={(event) => choose(event.currentTarget.value || null, false)}
        >
          <option value="">{es.dictionary.inherit}</option>
          {(library?.dictionary_items ?? []).map(({ dictionary }) => (
            <option key={dictionary.id} value={dictionary.id}>
              {dictionary.name}
            </option>
          ))}
        </select>
      </label>
      {current?.series && selected && (
        <Button disabled={pending} size={28} variant="ghost" onClick={() => choose(selected, true)}>
          {es.library.useAlways}
        </Button>
      )}
      {pending && <p role="status">{es.dictionary.saving}</p>}
      <DictionaryFeedback library={library} />
    </div>
  );
}
