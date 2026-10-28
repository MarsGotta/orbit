import type { DaemonPort } from "../bridge/port";
import { CopyProvider } from "../i18n/CopyProvider";
import { primaryAgentName } from "../features/assistant/agents";
import { useAppStore } from "./use-app-store";
import { AppRoot } from "./AppRoot";
export type { AppContentProps } from "./AppRoot";

export function App({ port }: { port: DaemonPort }) {
  const app = useAppStore(port);
  return (
    <CopyProvider
      name={app.snapshot ? primaryAgentName(app.snapshot) : undefined}
      choice={app.snapshot?.settings.language}
    >
      <AppRoot port={port} app={app} />
    </CopyProvider>
  );
}
