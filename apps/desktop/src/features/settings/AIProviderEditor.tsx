import { getCopy } from "orbit-ui/i18n";
import { useCopy } from "orbit-ui/i18n/CopyProvider";
import { useState } from "react";
import type { AiModel } from "orbit-ui/bridge/generated/AiModel";
import type { AiProvider } from "orbit-ui/bridge/generated/AiProvider";
import { Button } from "orbit-ui/design/Button";
import { Field } from "orbit-ui/design/Field";

import {
  MAX_AI_PROVIDER_NAME_CHARS,
  MAX_AI_PROVIDER_URL_CHARS,
  MAX_AI_PROVIDER_KEY_CHARS,
} from "./ai-limits";
import { ApiModels } from "./ApiModels";
import { List, Row } from "./layout";
import type { useAISettings } from "./use-ai-settings";
import styles from "./SettingsView.module.css";

type View = ReturnType<typeof useAISettings>;
const routerUrl = "https://openrouter.ai/api/v1";

type Props = { view: View; existing?: AiProvider; onClose: () => void };

function newProviderId(kind: string): string {
  if (kind === "openrouter") return "openrouter";
  return `${kind === "api" ? "api" : "local"}-${window.crypto.randomUUID()}`;
}

function useProviderForm({ view, existing, onClose }: Props) {
  const [name, setName] = useState(existing?.name ?? "");
  const [url, setUrl] = useState(existing?.base_url ?? "");
  const [key, setKey] = useState("");
  const [kind, setKind] = useState(existing?.kind ?? "local");
  const [models, setModels] = useState<AiModel[]>(
    existing?.kind === "api" ? existing.models : [{ id: "", capabilities: [] }],
  );
  const endpoint = kind === "local" || kind === "api";
  const named = models
    .map((model) => ({ ...model, id: model.id.trim() }))
    .filter((model) => model.id.length > 0);
  const ready =
    !endpoint ||
    (name.trim().length > 0 && url.trim().length > 0 && (kind !== "api" || named.length > 0));
  const save = async () => {
    const ok = await view.call({
      call: "ai_provider_save",
      provider_id: existing?.id ?? newProviderId(kind),
      provider: {
        name: kind === "openrouter" ? getCopy().settingsInline.openRouter : name,
        kind,
        base_url: kind === "openrouter" ? routerUrl : url,
        ...(key.length > 0 ? { api_key: key } : {}),
        ...(kind === "api" ? { models: named } : {}),
      },
    });
    setKey("");
    if (ok) onClose();
  };
  return {
    name,
    setName,
    url,
    setUrl,
    key,
    setKey,
    kind,
    setKind,
    models,
    setModels,
    endpoint,
    ready,
    save,
  };
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
  const inlineCopy = useCopy().settingsInline;
  const copy = useCopy().extras.aiCopy;
  const { view, existing, onClose } = props;
  const form = useProviderForm(props);
  const { key, setKey, kind, setKind, save } = form;
  return (
    <List busy={view.busy}>
      <Row title={inlineCopy.provider}>
        <div className={styles.select}>
          <select
            aria-label={inlineCopy.providerType}
            disabled={view.busy || existing !== undefined}
            value={kind}
            onChange={(e) => setKind(e.target.value)}
          >
            <option value="local">{copy.local}</option>
            <option value="openrouter">{inlineCopy.openRouter}</option>
            <option value="api">{copy.api}</option>
          </select>
        </div>
        {form.endpoint && <EndpointFields form={form} busy={view.busy} />}
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
        {kind === "api" && (
          <ApiModels models={form.models} onChange={form.setModels} busy={view.busy} />
        )}
        <Button disabled={view.busy || !form.ready} onClick={() => void save()}>
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
          {inlineCopy.cancel}
        </Button>
        {view.error.length > 0 && <p role="alert">{view.error}</p>}
      </Row>
    </List>
  );
}
