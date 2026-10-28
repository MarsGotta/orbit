import { lookupCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { AiFunction } from "../../bridge/generated/AiFunction";
import type { AiProvider } from "../../bridge/generated/AiProvider";

import { Group, List, Row } from "./layout";
import type { useAISettings } from "./use-ai-settings";
import styles from "./SettingsView.module.css";

type View = ReturnType<typeof useAISettings>;
interface FunctionRowProps {
  fn: AiFunction;
  providers: AiProvider[];
  view: View;
}

function FunctionRow({ fn, providers, view }: FunctionRowProps) {
  const inlineCopy = useCopy().settingsInline;
  const copy = useCopy().extras.aiCopy;
  const options = providers.flatMap((provider) =>
    provider.models
      .filter((model) => model.capabilities.includes(fn.capability))
      .map((model) => ({
        provider_id: provider.id,
        model: model.id,
        label: `${provider.name} · ${model.id}`,
      })),
  );
  const selected = JSON.stringify([fn.provider_id, fn.model]);
  const operator = JSON.stringify(["operator", "default"]);
  return (
    <Row title={lookupCopy(copy.functions, fn.id) ?? fn.id} help={fn.privacy}>
      <div className={`${styles.select} ${styles.aiSelect}`}>
        <select
          aria-label={lookupCopy(copy.functions, fn.id) ?? fn.id}
          disabled={view.busy}
          value={fn.provider_id === "operator" ? operator : selected}
          onChange={(e) => {
            const [provider_id, model] = JSON.parse(e.target.value) as [string, string];
            void view.call({
              call: "ai_function_save",
              function: fn.id,
              choice: { provider_id, model },
            });
          }}
        >
          <option value={operator}>
            {copy.operator}
            {fn.provider_id === "operator" ? ` · ${fn.model}` : ""}
          </option>
          {options.map((option) => (
            <option
              key={JSON.stringify([option.provider_id, option.model])}
              value={JSON.stringify([option.provider_id, option.model])}
            >
              {option.label}
            </option>
          ))}
        </select>
      </div>
      {view.errorPlace === fn.id && view.error.length > 0 && <p role="alert">{view.error}</p>}
      {fn.id === "vault" && <p>{inlineCopy.vaultFunctionHelp}</p>}
      {fn.id === "assistants" && <p>{inlineCopy.assistantFunctionHelp}</p>}
    </Row>
  );
}
export function AIFunctions({ view }: { view: View }) {
  const inlineCopy = useCopy().settingsInline;
  if (view.settings === null) return null;
  return (
    <Group label={inlineCopy.functions} help={inlineCopy.functionsHelp}>
      <List busy={view.busy}>
        {view.settings.functions.map((fn) => (
          <FunctionRow key={fn.id} fn={fn} providers={view.settings?.providers ?? []} view={view} />
        ))}
        <Row title={inlineCopy.diarization} help={inlineCopy.diarizationHelp} />
      </List>
    </Group>
  );
}
