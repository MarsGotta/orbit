import type { AppCommand } from "../../bridge/generated";
import type { CallPreference } from "../../bridge/generated/CallPreference";
import { Card } from "../../design/Card";
import { SectionLabel } from "../../design/SectionLabel";
import { es } from "../../i18n/es";
import styles from "./SettingsView.module.css";

const OPTIONS: { value: CallPreference; label: string }[] = [
  { value: "notify", label: es.callSettings.notify },
  { value: "record", label: es.callSettings.recordWithoutAsking },
  { value: "nothing", label: es.callSettings.nothing },
];

/**
 * P9-11 «Al detectar una llamada»: one preference the daemon keeps. The
 * macOS microphone permission is the decision; this only chooses what Orbit
 * does then. Unknown (`null`) checks nothing and says nothing (SIM-24): it
 * never pretends «Avisarme»; an unreadable one is the catalogue's notice.
 */
export function CallPreferenceSection({
  preference,
  send,
}: {
  preference: CallPreference | null;
  send: (command: AppCommand) => void;
}) {
  return (
    <section aria-label={es.callSettings.section} className={styles.section}>
      <SectionLabel>{es.callSettings.section}</SectionLabel>
      <Card>
        <div className={styles.rowCopy}>
          <strong id="call-preference-title">{es.callSettings.preferenceTitle}</strong>
        </div>
        <div aria-labelledby="call-preference-title" className={styles.choices} role="radiogroup">
          {OPTIONS.map((option) => (
            <label className={styles.choice} key={option.value}>
              <input
                checked={preference === option.value}
                name="call-preference"
                onChange={() => send({ command: "call_preference", preference: option.value })}
                type="radio"
                value={option.value}
              />
              <span>{option.label}</span>
            </label>
          ))}
        </div>
      </Card>
    </section>
  );
}
