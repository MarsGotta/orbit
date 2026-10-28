import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useFeedback } from "../../design/feedback";
import { Field } from "../../design/Field";
import { Segmented } from "../../design/Segmented";
import { Skeleton } from "../../design/Skeleton";
import { Switch } from "../../design/Switch";
import { toggleTheme, useTheme, type Theme } from "../../design/theme";

import { useHeldChoice } from "../../state/use-held-choice";
import { Group, List, Row, SectionHeading } from "./layout";
import { LanguageSetting } from "./LanguageSetting";
import { ThisMac } from "./ThisMac";
import { Updates } from "./Updates";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => unknown;
type Settings = AppSnapshot["settings"];

/** «Tu nombre», saved when the person leaves the field (or presses ↵). */
function NameField({ send, settings }: { send: Send; settings: Settings }) {
  const base = useCopy();
  const known = settings.profile?.name ?? "";
  const [name, setName] = useState(known);
  const { feedback, track } = useFeedback();
  useEffect(() => setName(known), [known]);
  if (settings.profile === undefined)
    return (
      <div aria-busy="true" className={styles.padded}>
        <Skeleton widths={["20%", "70%"]} />
      </div>
    );
  return (
    <div className={styles.padded}>
      <Field
        help={base.settings.yourNameHelp}
        label={base.settings.yourName}
        onBlur={() => {
          const next = name.trim() || null;
          if (next === (settings.profile?.name ?? null)) return;
          // A refused order (`false`) is not «✓ Guardado»: its notice says why.
          const sent = Promise.resolve(send({ command: "profile_set", name: next }));
          track(
            sent.then((taken) => (taken === false ? Promise.reject(new Error("refused")) : taken)),
          );
        }}
        onChange={(event) => setName(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
        placeholder={base.settings.yourNamePlaceholder}
        saved={feedback === "done"}
        value={name}
      />
    </div>
  );
}

/** Oscuro / Claro: fixes the theme on this Mac, as the top bar's button does. */
function Appearance() {
  const base = useCopy();
  const theme = useTheme();
  return (
    <Segmented<Theme>
      label={base.settings.theme}
      onChange={(next) => {
        if (next !== theme) toggleTheme();
      }}
      options={[
        { value: "dark", label: base.settings.dark },
        { value: "light", label: base.settings.light },
      ]}
      value={theme}
    />
  );
}

/** «Abrir Orbit al iniciar sesión», shown as chosen until macOS confirms it. */
function LoginItem({ send, settings }: { send: Send; settings: Settings }) {
  const base = useCopy();
  const loginItem = useHeldChoice(settings.login_item === "on", send);
  return (
    <Row
      aside={
        <Switch
          checked={loginItem.shown}
          label={base.settings.openAtLogin}
          onCheckedChange={(on) => loginItem.choose(on, { command: "login_item", on })}
        />
      }
      help={settings.login_item === "unknown" ? base.settings.loginUnknown : undefined}
      title={base.settings.openAtLogin}
    />
  );
}

/** Ajustes › General: the account, this Mac, the look, the start and the updates. */
export function GeneralSection({
  onReopenOnboarding,
  send,
  snapshot,
}: {
  onReopenOnboarding: (mode?: "pair") => void;
  send: Send;
  snapshot: AppSnapshot;
}) {
  const base = useCopy();
  const settings = snapshot.settings;
  const availability = snapshot.status_availability?.state;
  const statusKnown = availability !== "unknown" && availability !== "absent";
  return (
    <>
      <SectionHeading
        lead={base.settings.leads.general}
        title={base.shell.settingsSections.general}
      />
      <Group label={base.settings.account}>
        <List>
          <NameField send={send} settings={settings} />
          <ThisMac onReopen={onReopenOnboarding} settings={settings} statusKnown={statusKnown} />
          <div className={styles.padded}>
            <button className={styles.link} onClick={() => onReopenOnboarding()} type="button">
              {base.settings.reopenOnboarding}
            </button>
          </div>
        </List>
      </Group>
      <LanguageSetting send={send} />
      <Group label={base.settings.appearance}>
        <Appearance />
      </Group>
      <Group label={base.settings.startup}>
        <List>
          <LoginItem send={send} settings={settings} />
        </List>
      </Group>
      <Group label={base.settings.updates}>
        <Updates send={send} settings={settings} />
      </Group>
    </>
  );
}
