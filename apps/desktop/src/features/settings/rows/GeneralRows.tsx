import type { AppView as AppSnapshot } from "../../../state/app-view";
import { useEffect, useState } from "react";
import type { AppCommand, SettingsSnapshot } from "../../../bridge/generated";
import { Button } from "../../../design/Button";
import { Card } from "../../../design/Card";
import { SectionLabel } from "../../../design/SectionLabel";
import { Toggle } from "../../../design/Toggle";
import { TextInput } from "../../../design/TextInput";
import { es } from "../../../i18n/es";
import { useUiStore } from "../../../state/ui";
import { AccountPairingControls } from "../AccountPairingControls";
import { LastInstallation, UpdateStatus } from "./UpdateStatus";
import styles from "../SettingsView.module.css";

interface CommandProps {
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
  statusAvailability?: AppSnapshot["status_availability"];
}

export function AccountSection({
  onReopen,
  send,
  settings,
  statusAvailability,
}: {
  onReopen: (mode?: "pair") => void;
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
  statusAvailability?: AppSnapshot["status_availability"];
}) {
  const [name, setName] = useState(settings.profile?.name ?? "");
  useEffect(() => setName(settings.profile?.name ?? ""), [settings.profile?.name]);
  const saveName = () => send({ command: "profile_set", name: name.trim() || null });
  return (
    <section aria-label={es.settings.account} className={styles.section}>
      <SectionLabel>{es.settings.account}</SectionLabel>
      <Card>
        <div className={styles.details}>
          <label className={styles.nameField}>
            <span>{es.settings.yourName}</span>
            <TextInput
              onBlur={saveName}
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              placeholder={es.settings.yourNamePlaceholder}
              value={name}
            />
          </label>
        </div>
        <AccountPairingControls
          hub={settings.hub}
          onReopen={onReopen}
          statusAvailability={statusAvailability}
        />
      </Card>
    </section>
  );
}

export function StartupSection({ send, settings }: CommandProps) {
  return (
    <section aria-label={es.settings.startup} className={styles.section}>
      <SectionLabel>{es.settings.startup}</SectionLabel>
      <Card>
        <div className={styles.inlineControl}>
          <strong>{es.settings.openAtLogin}</strong>
          <Toggle
            checked={settings.login_item === "on"}
            label={es.settings.openAtLogin}
            onCheckedChange={(on) => send({ command: "login_item", on })}
          />
        </div>
      </Card>
    </section>
  );
}

export function AppearanceSection() {
  const theme = useUiStore((state) => state.theme);
  const toggleTheme = useUiStore((state) => state.toggleTheme);
  return (
    <section aria-label={es.settings.appearance} className={styles.section}>
      <SectionLabel>{es.settings.appearance}</SectionLabel>
      <Card>
        <div className={styles.inlineControl}>
          <div>
            <strong>{es.settings.theme}</strong>
            <span>{theme === "dark" ? es.settings.dark : es.settings.light}</span>
          </div>
          <Button onClick={toggleTheme} variant="ghost">
            {es.shell.changeTheme}
          </Button>
        </div>
      </Card>
    </section>
  );
}

export function UpdatesSection({ send, settings }: CommandProps) {
  const busy = settings.update.state === "checking" || settings.update.state === "installing";
  return (
    <section aria-label={es.settings.updates} className={styles.section}>
      <SectionLabel>{es.settings.updates}</SectionLabel>
      <Card>
        <p>{es.settings.version(settings.versions.app)}</p>
        <UpdateStatus settings={settings} />
        <LastInstallation settings={settings} />
        {Boolean(settings.update.install_blocked) && <p>{settings.update.install_blocked}</p>}
        <div className={styles.actions}>
          <Button
            disabled={busy || settings.update.state === "disabled"}
            onClick={() => send({ command: "update_check" })}
          >
            {es.settings.searchUpdate}
          </Button>
          {settings.update.state === "available" && (
            <Button
              disabled={Boolean(settings.update.install_blocked)}
              onClick={() => send({ command: "update_install" })}
              variant="accent"
            >
              {es.settings.installRestart}
            </Button>
          )}
        </div>
      </Card>
    </section>
  );
}

export function PrivacySection({ send, settings, statusAvailability }: CommandProps) {
  // An unknown state says nothing: the button still stops.
  const unknown = statusAvailability?.state === "unknown" || statusAvailability?.state === "absent";
  return (
    <section aria-label={es.settings.privacy} className={styles.section}>
      <SectionLabel>{es.settings.privacy}</SectionLabel>
      <Card>
        {!unknown && (
          <p>{settings.privacy_stopped ? es.settings.privacyActive : es.settings.privacyArmed}</p>
        )}
        <Button
          onClick={() =>
            send({
              command: unknown || !settings.privacy_stopped ? "privacy_stop" : "rearm",
            })
          }
          variant={settings.privacy_stopped ? "accent" : "outline"}
        >
          {settings.privacy_stopped ? es.settings.rearm : es.settings.stopNow}
        </Button>
        {/* Only a build that carries a Sentry address offers it. */}
        {settings.error_reports !== undefined && (
          <div className={styles.inlineControl}>
            <div className={styles.rowCopy}>
              <strong>{es.settings.errorReports}</strong>
              <span>{es.settings.errorReportsHelp}</span>
            </div>
            <Toggle
              checked={settings.error_reports}
              label={es.settings.errorReports}
              onCheckedChange={(on) => send({ command: "error_reports", enabled: on })}
            />
          </div>
        )}
      </Card>
    </section>
  );
}
