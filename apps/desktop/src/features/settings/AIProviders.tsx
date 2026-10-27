import { useState } from "react";
import type { AiProvider } from "../../bridge/generated/AiProvider";
import type { AiModel } from "../../bridge/generated/AiModel";
import { Button } from "../../design/Button";
import { aiCopy as copy } from "../../i18n/es/ai";
import { Group, List, Row } from "./layout";
import { AIProviderEditor } from "./AIProviderEditor";
import type { useAISettings } from "./use-ai-settings";

type View = ReturnType<typeof useAISettings>;
function ModelCorrection({ provider, view }: { provider: AiProvider; view: View }) {
  const [models, setModels] = useState<AiModel[]>(provider.models);
  return (
    <details>
      <summary>Corregir capacidades detectadas</summary>
      {models.map((model, index) => (
        <fieldset key={model.id}>
          <legend>{model.id}</legend>
          {Object.entries(copy.capabilities).map(([id, label]) => (
            <label key={id}>
              <input
                type="checkbox"
                checked={model.capabilities.includes(id)}
                onChange={(e) =>
                  setModels(
                    models.map((item, at) =>
                      at !== index
                        ? item
                        : {
                            ...item,
                            capabilities: e.target.checked
                              ? [...item.capabilities, id]
                              : item.capabilities.filter((c) => c !== id),
                          },
                    ),
                  )
                }
              />
              {label}
            </label>
          ))}
        </fieldset>
      ))}
      <Button
        disabled={view.busy}
        onClick={() =>
          void view.call({
            call: "ai_provider_save",
            provider_id: provider.id,
            provider: {
              name: provider.name,
              kind: provider.kind,
              base_url: provider.base_url ?? "",
              models,
            },
          })
        }
      >
        Guardar capacidades
      </Button>
    </details>
  );
}
export function AIProviders({
  view,
  initiallyOpen = false,
}: {
  view: View;
  initiallyOpen?: boolean;
}) {
  const [editing, setEditing] = useState<AiProvider | "new" | null>(initiallyOpen ? "new" : null);
  return (
    <Group label="Proveedores">
      <List>
        <Row
          title="En tu hub"
          help="Whisper y pyannote · Sin clave. Sus valores los configura el operador."
        />
        {view.settings?.providers
          .filter((p) => p.kind !== "hub")
          .map((p) => (
            <Row
              key={p.id}
              title={p.name}
              help={`${p.base_url ?? ""} · ${p.key_configured ? "Clave guardada en el hub" : "Sin clave"}`}
            >
              <p>{p.models.map((m) => m.id).join(", ")}</p>
              <Button disabled={view.busy} onClick={() => setEditing(p)}>
                Editar {p.name}
              </Button>
              <ModelCorrection key={JSON.stringify(p.models)} provider={p} view={view} />
            </Row>
          ))}
        <Row
          title={copy.local}
          help="Añade varios endpoints compatibles con OpenAI. Aunque estén en tu red, los datos salen del hub si la URL no es loopback."
        />
        <Row
          title={copy.cloud}
          help="OpenRouter · Gemini, OpenAI, Mistral y Azure llegarán más adelante."
        />
        <Row
          title={
            <Button disabled={view.busy} onClick={() => setEditing("new")}>
              {copy.add}
            </Button>
          }
        />
      </List>
      {editing === null && view.errorPlace === "provider" && view.error.length > 0 && (
        <p role="alert">{view.error}</p>
      )}
      {editing !== null && (
        <AIProviderEditor
          key={editing === "new" ? "new" : editing.id}
          existing={editing === "new" ? undefined : editing}
          view={view}
          onClose={() => setEditing(null)}
        />
      )}
    </Group>
  );
}
