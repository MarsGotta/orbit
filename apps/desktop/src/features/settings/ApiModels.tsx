import { useCopy } from "orbit-ui/i18n/CopyProvider";
import type { AiModel } from "orbit-ui/bridge/generated/AiModel";
import { Button } from "orbit-ui/design/Button";
import { Field } from "orbit-ui/design/Field";

import styles from "./SettingsView.module.css";

type ChecksProps = {
  capabilities: string[];
  onChange: (capabilities: string[]) => void;
  disabled?: boolean;
};

/** One checkbox per capability, named by its label. */
export function CapabilityChecks({ capabilities, onChange, disabled }: ChecksProps) {
  const copy = useCopy().extras.aiCopy;
  return (
    <>
      {Object.entries(copy.capabilities).map(([id, label]) => (
        <label key={id}>
          <input
            type="checkbox"
            disabled={disabled}
            checked={capabilities.includes(id)}
            onChange={(e) =>
              onChange(
                e.target.checked ? [...capabilities, id] : capabilities.filter((c) => c !== id),
              )
            }
          />
          {label}
        </label>
      ))}
    </>
  );
}

type ModelsProps = { models: AiModel[]; onChange: (models: AiModel[]) => void; busy: boolean };

/**
 * The models an OpenAI-compatible API offers, as the person names them, each
 * with what it does. The hub checks them against its catalogue, never probes
 * them. A model is removed by clearing its name.
 */
export function ApiModels({ models, onChange, busy }: ModelsProps) {
  const copy = useCopy().extras.aiCopy;
  const change = (index: number, model: AiModel) =>
    onChange(models.map((item, at) => (at === index ? model : item)));
  return (
    <>
      <p className={styles.rowHelp}>{copy.modelsHelp}</p>
      {models.map((model, index) => (
        // Rows are only appended, never removed or reordered.
        <fieldset key={index} aria-label={model.id.trim() || undefined}>
          <Field
            disabled={busy}
            label={copy.model}
            value={model.id}
            onChange={(e) => change(index, { ...model, id: e.target.value })}
          />
          <CapabilityChecks
            disabled={busy}
            capabilities={model.capabilities}
            onChange={(capabilities) => change(index, { ...model, capabilities })}
          />
        </fieldset>
      ))}
      <Button
        disabled={busy}
        variant="ghost"
        onClick={() => onChange([...models, { id: "", capabilities: [] }])}
      >
        {copy.addModel}
      </Button>
    </>
  );
}
