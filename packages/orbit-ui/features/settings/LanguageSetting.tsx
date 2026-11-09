import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import { useCopy, useLanguage } from "../../i18n/CopyProvider";
import type { LanguageChoice } from "../../i18n/language";
import { Group, List, Row } from "./layout";
import styles from "./SettingsView.module.css";

export function LanguageSetting({ send }: { send: (command: AppCommand) => unknown }) {
  const copy = useCopy();
  const { choice, select } = useLanguage();
  const id = useId();
  const choose = (language: LanguageChoice) => {
    void Promise.resolve(send({ command: "language_set", language })).then(
      (accepted) => {
        if (accepted !== false) select?.(language);
      },
      () => undefined,
    );
  };
  return (
    <Group label={copy.settings.language}>
      <List>
        <Row
          title={<label htmlFor={id}>{copy.settings.language}</label>}
          aside={
            <div className={styles.select}>
              <select
                id={id}
                value={choice}
                onChange={(event) => choose(event.target.value as LanguageChoice)}
              >
                <option value="system">{copy.settings.systemLanguage}</option>
                <option value="es">{copy.settings.spanish}</option>
                <option value="en">{copy.settings.english}</option>
              </select>
            </div>
          }
        />
      </List>
    </Group>
  );
}
