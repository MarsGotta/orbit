import { Button } from "../../design/Button";
import { Field } from "../../design/Field";
import { transcription as copy } from "../../i18n/es/transcription";
import type { TranscriptionModel } from "../../bridge/generated/TranscriptionModel";
import type { AgentPort } from "./agents/use-agent-config";
import { Group, List, Row } from "./layout";
import { useTranscriptionSettings } from "./use-transcription-settings";
import styles from "./SettingsView.module.css";

type View = ReturnType<typeof useTranscriptionSettings>;

function KeyField({ view }: { view: View }) {
  return (
    <Row title={copy.mai} help={copy.privacy}>
      <Field
        label={copy.key}
        type="password"
        autoComplete="off"
        value={view.key}
        help={view.settings?.key_configured === true ? copy.keyStored : copy.keyHelp}
        disabled={view.busy}
        onChange={(event) => view.setKey(event.target.value)}
      />
    </Row>
  );
}

export function TranscriptionSettings({ agent }: { agent?: AgentPort }) {
  const view = useTranscriptionSettings({ agent });
  return (
    <Group label={copy.title} help={copy.scope}>
      <List busy={view.busy}>
        <Row title={copy.label}>
          {view.settings ? (
            <div className={styles.select}>
              <select
                aria-label={copy.label}
                disabled={view.busy}
                value={view.model}
                onChange={(event) => {
                  view.setModel(event.target.value as TranscriptionModel);
                  view.setFeedback("");
                }}
              >
                <option value="whisper">{copy.whisper}</option>
                <option value="mai">{copy.mai}</option>
              </select>
            </div>
          ) : (
            <p>{view.error || (agent ? copy.loading : copy.unavailable)}</p>
          )}
        </Row>
        {view.model === "mai" && <KeyField view={view} />}
        <Row
          title={
            <Button disabled={!view.settings || view.busy} onClick={() => void view.save()}>
              {view.busy ? copy.saving : copy.save}
            </Button>
          }
        >
          {view.error && <p role="alert">{view.error}</p>}
          {view.feedback && <p role="status">{view.feedback}</p>}
        </Row>
      </List>
    </Group>
  );
}
