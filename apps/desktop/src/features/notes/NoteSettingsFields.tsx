import { useCopy } from "../../i18n/CopyProvider";
import type { ReactNode } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Icon } from "../../design/Icon";
import { MultiSelect } from "../../design/MultiSelect";
import { Switch } from "../../design/Switch";

import { useUiStore } from "../../state/ui";
import { chosen, dictionaryOptions, meetingDictionaries } from "./dictionary-choice";
import { languageName } from "./notes-format";
import { type Choice, LANGUAGES, templateName } from "./note-settings";
import styles from "./Note.module.css";

// The fields of «Cómo escribe Orbit esta nota»: template, meeting
// language, word dictionaries and «Usar lo mismo en toda la serie».

function Select({
  label,
  value,
  onChange,
  disabled,
  children,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <label className={styles.menuField}>
      <span>{label}</span>
      <span className={styles.select}>
        <select
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          value={value}
        >
          {children}
        </select>
        <Icon name="chevron" size={14} />
      </span>
    </label>
  );
}

type FieldProps = {
  draft: Choice;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  set: (change: Partial<Choice>) => void;
};

function TemplateField({ draft, library, note, set }: FieldProps) {
  const es = useCopy();
  const templates = library?.templates.state === "available" ? library.items : [];
  const known = templates.some((item) => item.id === draft.template);
  return (
    <>
      <Select
        disabled={templates.length === 0}
        label={es.noteScreen.template}
        onChange={(template) => set({ template })}
        value={draft.template}
      >
        {!known && (
          <option value={draft.template}>{templateName(library, draft.template, note)}</option>
        )}
        {templates.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name}
          </option>
        ))}
      </Select>
      {templates.length === 0 && (
        <p className={styles.menuHelp} role="status">
          {library?.templates.state === "failed" || library?.read === "failed"
            ? es.noteScreen.templatesFailed
            : es.noteScreen.templatesUnread}
        </p>
      )}
    </>
  );
}

function LanguageField({ draft, set }: FieldProps) {
  const es = useCopy();
  const language = draft.language;
  if (language === null) return null;
  const languages = LANGUAGES.includes(language) ? LANGUAGES : [...LANGUAGES, language];
  return (
    <Select
      label={es.noteScreen.language}
      onChange={(value) => set({ language: value })}
      value={language}
    >
      {languages.map((code) => (
        <option key={code} value={code}>
          {languageName(code)}
        </option>
      ))}
    </Select>
  );
}

/** Several dictionaries at once (annex, change 6); dictionaries are made by correcting words. */
function DictionaryField({ draft, library, note, set }: FieldProps) {
  const es = useCopy();
  const meeting = meetingDictionaries(note);
  if (meeting === null) return null;
  if (library?.dictionaries?.state !== "available")
    return (
      <p className={styles.menuHelp} role="status">
        {es.noteScreen.dictionariesUnread}
      </p>
    );
  return (
    <MultiSelect
      createLabel={es.noteScreen.manageDictionaries}
      label={es.noteScreen.dictionaries}
      onChange={(next) => set({ dictionaries: chosen(note, draft.dictionaries, next) })}
      onCreate={() => useUiStore.getState().openSettings("reuniones")}
      options={dictionaryOptions(library, meeting)}
      placeholder={es.noteScreen.noDictionaries}
      selected={draft.dictionaries}
    />
  );
}

export function Fields(props: FieldProps) {
  const es = useCopy();
  return (
    <div className={styles.menuFields}>
      <TemplateField {...props} />
      <LanguageField {...props} />
      <DictionaryField {...props} />
      {props.note.series && (
        <Switch
          checked={props.draft.series}
          label={es.noteScreen.useForSeries}
          onCheckedChange={(series) => props.set({ series })}
        >
          <span className={styles.switchLabel}>{es.noteScreen.useForSeries}</span>
        </Switch>
      )}
    </div>
  );
}
