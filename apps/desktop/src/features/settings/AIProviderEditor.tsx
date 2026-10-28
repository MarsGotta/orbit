import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AiProvider } from "../../bridge/generated/AiProvider";
import { Button } from "../../design/Button";
import { Field } from "../../design/Field";

import {
  MAX_AI_PROVIDER_NAME_CHARS,
  MAX_AI_PROVIDER_URL_CHARS,
  MAX_AI_PROVIDER_KEY_CHARS,
} from "./ai-limits";
import { List, Row } from "./layout";
import type { useAISettings } from "./use-ai-settings";
import styles from "./SettingsView.module.css";

type View = ReturnType<typeof useAISettings>;
const routerUrl = "https://openrouter.ai/api/v1";

type Props = { view: View; existing?: AiProvider; onClose: () => void };

function useProviderForm({ view, existing, onClose }: Props) {
  const [name, setName] = useState(existing?.name ?? "");
  const [url, setUrl] = useState(existing?.base_url ?? "");
  const [key, setKey] = useState("");
  const [kind, setKind] = useState(existing?.kind ?? "local");
  const save = async () => {
    const providerId =
      existing?.id ??
      (kind === "openrouter" ? "openrouter" : `local-${window.crypto.randomUUID()}`);
    const ok = await view.call({
      call: "ai_provider_save",
      provider_id: providerId,
      provider: {
        name: kind === "openrouter" ? "OpenRouter" : name,
        kind,
        base_url: kind === "openrouter" ? routerUrl : url,
        ...(key.length > 0 ? { api_key: key } : {}),
      },
    });
    setKey("");
    if (ok) onClose();
  };
  return { name, setName, url, setUrl, key, setKey, kind, setKind, save };
}
type Form = ReturnType<typeof useProviderForm>;

function EndpointFields({ form, busy }: { form: Form; busy: boolean }) {
  const copy = useCopy().extras.aiCopy;
  return (
    <>
      <Field
        disabled={busy}
        label={copy.providerName}
        value={form.name}
        maxLength={MAX_AI_PROVIDER_NAME_CHARS}
        onChange={(e) => form.setName(e.target.value)}
      />
      <Field
        disabled={busy}
        label={copy.baseUrl}
        value={form.url}
        type="url"
        maxLength={MAX_AI_PROVIDER_URL_CHARS}
        onChange={(e) => form.setUrl(e.target.value)}
      />
    </>
  );
}

export function AIProviderEditor(props: Props) {
  const copy = useCopy().extras.aiCopy;
  const { view, existing, onClose } = props;
  const form = useProviderForm(props);
  const { name, url, key, setKey, kind, setKind, save } = form;
  return (
    <List busy={view.busy}>
      <Row title="Proveedor">
        <div className={styles.select}>
          <select
            aria-label="Tipo de proveedor"
            disabled={view.busy || existing !== undefined}
            value={kind}
            onChange={(e) => setKind(e.target.value)}
          >
            <option value="local">{copy.local}</option>
            <option value="openrouter">OpenRouter</option>
          </select>
        </div>
        {kind === "local" && <EndpointFields form={form} busy={view.busy} />}
        <Field
          disabled={view.busy}
          label={copy.key}
          help={copy.keyHelp}
          value={key}
          type="password"
          autoComplete="off"
          maxLength={MAX_AI_PROVIDER_KEY_CHARS}
          onChange={(e) => setKey(e.target.value)}
        />
        <Button
          disabled={
            view.busy || (kind === "local" && (name.trim().length === 0 || url.trim().length === 0))
          }
          onClick={() => void save()}
        >
          {view.busy ? copy.saving : copy.save}
        </Button>
        <Button
          disabled={view.busy}
          variant="ghost"
          onClick={() => {
            setKey("");
            onClose();
          }}
        >
          Cancelar
        </Button>
        {view.error.length > 0 && <p role="alert">{view.error}</p>}
      </Row>
    </List>
  );
}
