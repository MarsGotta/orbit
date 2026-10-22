import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { DictionarySnapshot } from "../../bridge/generated/DictionarySnapshot";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { Button } from "../../design/Button";
import { TextInput } from "../../design/TextInput";
import { Card } from "../../design/Card";
import { SectionLabel } from "../../design/SectionLabel";
import { es } from "../../i18n/es";
import { DictionaryFeedback, sendDictionary } from "../notes/DictionaryPicker";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => void;
function DictionaryEditor({ item, send }: { item: DictionarySnapshot; send: Send }) {
  const { dictionary, pending } = item;
  const [name, setName] = useState(dictionary.name);
  const series = dictionary.series
    .map((series) => series.title || series.calendar_item_id)
    .join(", ");
  return (
    <Card>
      <form
        className={styles.inlineControl}
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim())
            sendDictionary(send, {
              action: "save",
              dictionary_id: dictionary.id,
              name: name.trim(),
            });
        }}
      >
        <label className={styles.nameField}>
          <span>{es.dictionary.name}</span>
          <TextInput value={name} onChange={(event) => setName(event.currentTarget.value)} />
        </label>
        <Button type="submit" disabled={pending || !name.trim() || name === dictionary.name}>
          {es.notes.save}
        </Button>
        {pending && <p role="status">{es.dictionary.saving}</p>}
      </form>
      <p>{series ? es.dictionary.series(series) : es.dictionary.noSeries}</p>
      {dictionary.terms.length === 0 ? (
        <p>{es.dictionary.emptyTerms}</p>
      ) : (
        <ul>
          {dictionary.terms.map((entry) => (
            <li key={entry.id}>
              <strong>{entry.correct}</strong> ← {entry.wrong.join(", ")}{" "}
              <Button
                aria-label={es.dictionary.remove(entry.correct)}
                size={28}
                variant="ghost"
                disabled={pending}
                onClick={() =>
                  sendDictionary(send, {
                    action: "remove",
                    dictionary_id: dictionary.id,
                    entry_id: entry.id,
                  })
                }
              >
                {es.dictionary.removeEntry}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
export function MeetingDictionaries({
  library,
  send,
}: {
  library?: NoteLibrarySnapshot | null;
  send: Send;
}) {
  return (
    <section className={`${styles.section} ${styles.dictionary}`}>
      <SectionLabel>{es.dictionary.plural}</SectionLabel>
      <DictionaryFeedback library={library} />
      {library?.dictionaries?.state === "available" &&
        (library.dictionary_items?.length ? (
          library.dictionary_items.map((item) => (
            <DictionaryEditor
              key={`${item.dictionary.id}:${item.dictionary.name}`}
              item={item}
              send={send}
            />
          ))
        ) : (
          <p>{es.dictionary.empty}</p>
        ))}
    </section>
  );
}
