import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { DictionarySnapshot } from "../../bridge/generated/DictionarySnapshot";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import { Button } from "../../design/Button";

import { DictionaryFeedback, sendDictionary } from "../notes/DictionaryPicker";
import { Group, List, SkeletonRows } from "./layout";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => void;

/** The new name, typed in place: «Guardar», or Esc and «Cancelar» to leave it. */
function RenameForm({
  item,
  name,
  onName,
  send,
}: {
  item: DictionarySnapshot;
  name: string;
  onName: (name: string | null) => void;
  send: Send;
}) {
  const es = useCopy();
  const { dictionary } = item;
  return (
    <form
      className={styles.dictionaryHead}
      onSubmit={(event) => {
        event.preventDefault();
        if (!name.trim()) return;
        sendDictionary(send, { action: "save", dictionary_id: dictionary.id, name: name.trim() });
        onName(null);
      }}
    >
      <input
        aria-label={es.dictionary.name}
        autoFocus
        className={styles.inlineInput}
        onChange={(event) => onName(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") onName(null);
        }}
        value={name}
      />
      <Button
        disabled={!name.trim() || name === dictionary.name}
        size={28}
        type="submit"
        variant="accent"
      >
        {es.notes.save}
      </Button>
      <Button onClick={() => onName(null)} size={28} variant="ghost">
        {es.controls.cancel}
      </Button>
    </form>
  );
}

/**
 * How many words a card shows before «Ver las N palabras»: enough to see
 * what the dictionary is about, while three cards still fit on one screen.
 */
const WORDS_SHOWN = 5;

/** The dictionary's name, its words and the series that always use it; «Cambiar nombre» renames it. */
function DictionaryName({ item, send }: { item: DictionarySnapshot; send: Send }) {
  const es = useCopy();
  const { dictionary, pending } = item;
  const [name, setName] = useState<string | null>(null);
  if (name !== null) return <RenameForm item={item} name={name} onName={setName} send={send} />;
  const series = dictionary.series
    .map((series) => series.title || series.calendar_item_id)
    .join(", ");
  return (
    <div className={styles.dictionaryHead}>
      <span className={styles.dictionaryName}>{dictionary.name}</span>
      <span className={styles.meta}>{es.dictionary.words(dictionary.terms.length)}</span>
      <span className={`${styles.meta} ${styles.push}`}>
        {series ? es.dictionary.series(series) : es.dictionary.noSeries}
      </span>
      {pending && (
        <span className={styles.meta} role="status">
          {es.dictionary.saving}
        </span>
      )}
      <Button
        aria-label={es.dictionary.renameLabel(dictionary.name)}
        disabled={pending}
        onClick={() => setName(dictionary.name)}
        size={28}
        variant="ghost"
      >
        {es.dictionary.rename}
      </Button>
    </div>
  );
}

/** One dictionary's card: its first words, and the rest on asking. */
function Dictionary({ item, send }: { item: DictionarySnapshot; send: Send }) {
  const es = useCopy();
  const { dictionary, pending } = item;
  const [all, setAll] = useState(false);
  const terms = all ? dictionary.terms : dictionary.terms.slice(0, WORDS_SHOWN);
  return (
    <div aria-label={dictionary.name} className={styles.list} role="group">
      <DictionaryName item={item} send={send} />
      {dictionary.terms.length === 0 && <p className={styles.empty}>{es.dictionary.emptyTerms}</p>}
      {terms.map((entry) => (
        <div className={styles.term} key={entry.id}>
          <span className={styles.rowTitle}>{entry.correct}</span>
          <span className={styles.meta}>{es.dictionary.insteadOf(entry.wrong.join(", "))}</span>
          <Button
            aria-label={es.dictionary.remove(entry.correct)}
            className={styles.push}
            disabled={pending}
            onClick={() =>
              sendDictionary(send, {
                action: "remove",
                dictionary_id: dictionary.id,
                entry_id: entry.id,
              })
            }
            size={28}
            variant="ghost"
          >
            {es.dictionary.removeEntry}
          </Button>
        </div>
      ))}
      {terms.length < dictionary.terms.length && (
        <div className={styles.term}>
          <Button onClick={() => setAll(true)} size={28} variant="ghost">
            {es.dictionary.seeAll(dictionary.terms.length)}
          </Button>
        </div>
      )}
    </div>
  );
}

/**
 * Settings › Meetings › Diccionarios: a card per dictionary, since a
 * meeting or a series can use several, each term with what it replaces
 * («en lugar de…») and «Quitar». Terms are added from a transcript;
 * here they are only renamed and removed.
 */
export function MeetingDictionaries({
  library,
  send,
}: {
  library?: NoteLibrarySnapshot | null;
  send: Send;
}) {
  const es = useCopy();
  const state = library?.dictionaries?.state;
  const items = library?.dictionary_items ?? [];
  return (
    <Group
      count={state === "available" && items.length > 0 ? String(items.length) : undefined}
      help={es.dictionary.several}
      label={es.settings.dictionaries}
    >
      <DictionaryFeedback library={library} />
      {(state === undefined || state === "unknown") && <SkeletonRows rows={3} />}
      {state === "available" &&
        (items.length > 0 ? (
          <div className={styles.cards}>
            {items.map((item) => (
              <Dictionary
                item={item}
                key={`${item.dictionary.id}:${item.dictionary.name}`}
                send={send}
              />
            ))}
          </div>
        ) : (
          <List>
            <p className={styles.empty}>{es.dictionary.empty}</p>
          </List>
        ))}
    </Group>
  );
}
