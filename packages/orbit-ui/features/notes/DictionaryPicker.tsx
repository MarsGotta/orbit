import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import type { DictionaryRequest } from "../../bridge/generated/DictionaryRequest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";

// What Settings › Meetings still takes from here: the note itself chooses
// its dictionary in «Cómo escribe Orbit esta nota» (NoteSettingsMenu.tsx).

export function sendDictionary(send: (command: AppCommand) => void, request: DictionaryRequest) {
  send({ command: "note_library", request: { action: "dictionary", request } });
}

export function DictionaryFeedback({ library }: { library?: NoteLibrarySnapshot | null }) {
  const es = useCopy();
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
