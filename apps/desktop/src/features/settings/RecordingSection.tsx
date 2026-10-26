import type { AppCommand } from "../../bridge/generated";
import type { CallPreference } from "../../bridge/generated/CallPreference";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { RadioList } from "../../design/RadioList";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es as base } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import { Group, List, SectionHeading } from "./layout";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => unknown;

const PREFERENCES: readonly CallPreference[] = ["notify", "record", "nothing"];

/**
 * «Cuando empieces una videollamada»: one preference the daemon keeps. The
 * macOS microphone permission is the decision; this only chooses what Orbit
 * does then. Not known yet (`null`) marks nothing: it never pretends
 * «Avisarme», and the person can still choose. A choice stays marked while
 * its order travels, and goes back if it is refused.
 */
export function CallPreferenceList({
  preference,
  send,
}: {
  preference: CallPreference | null;
  send: Send;
}) {
  const choice = useHeldChoice(preference, send);
  const options = PREFERENCES.map((value) => ({
    value,
    label: base.callSettings.options[value].label,
    help: base.callSettings.options[value].help,
  }));
  return (
    <div className={styles.radioCard}>
      <RadioList<CallPreference>
        label={base.settings.whenCallLabel}
        name="call-preference"
        onChange={(next) => choice.choose(next, { command: "call_preference", preference: next })}
        options={options}
        // No option matches an unknown preference, so none is checked.
        value={choice.shown ?? ("" as CallPreference)}
      />
    </div>
  );
}

/** Ajustes › Grabación y llamadas: what a call does, how long things last, the shortcuts. */
export function RecordingSection({ send, snapshot }: { send: Send; snapshot: AppSnapshot }) {
  const es = useSpanish();
  return (
    <>
      <SectionHeading
        lead={base.settings.leads.grabacion}
        title={base.shell.settingsSections.grabacion}
      />
      <Group label={base.settings.whenCall}>
        <CallPreferenceList preference={snapshot.meeting.call_preference} send={send} />
      </Group>
      <Group label={base.settings.shortcuts}>
        <List>
          {es.settings.shortcutList.map(([keys, action]) => (
            <div className={styles.shortcut} key={keys}>
              <kbd>{keys}</kbd>
              <span>{action}</span>
            </div>
          ))}
        </List>
      </Group>
    </>
  );
}
