import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand, SettingsSnapshot } from "../../bridge/generated";
import {
  AccountSection,
  AppearanceSection,
  PrivacySection,
  StartupSection,
  UpdatesSection,
} from "./GeneralSections";

interface Props {
  onReopenOnboarding: (mode?: "pair") => void;
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
  statusAvailability?: AppSnapshot["status_availability"];
}

export function GeneralSettings({ onReopenOnboarding, send, settings, statusAvailability }: Props) {
  return (
    <>
      <AccountSection
        onReopen={onReopenOnboarding}
        send={send}
        settings={settings}
        statusAvailability={statusAvailability}
      />
      <StartupSection send={send} settings={settings} />
      <AppearanceSection />
      <UpdatesSection send={send} settings={settings} />
      <PrivacySection send={send} settings={settings} statusAvailability={statusAvailability} />
    </>
  );
}
