import type { PrivacyCopy } from "../../bridge/generated/PrivacyCopy";

const sentences = {
  openrouter: "El {material} sale a {destination}.",
  network: "El {material} sale a {destination}, fuera de tu hub, por tu red o Internet.",
  hub: "El {material} se queda en tu hub.",
  operator: "Se usa la configuración del operador; el servicio puede enviar datos fuera del hub.",
  vault_legacy: "Modelo de Notas guardado por el operador; el trabajo del hub aún no se ejecuta.",
  unconfigured: "No hay ningún modelo del operador configurado para esta función.",
  hermes:
    "El {material} se envía a Hermes en {destination}; su configuración determina si los datos se envían a otro proveedor.",
};

const destinations = {
  microsoft_openrouter: "Microsoft a través de OpenRouter",
  openrouter_provider: "OpenRouter y el proveedor del modelo",
};

const models = {
  "Hermes listening": "Escucha de Hermes",
  "Hermes speech": "Voz de Hermes",
  "No model configured": "No hay ningún modelo configurado",
};

export const aiPrivacy = {
  sentences,
  destinations,
  models,
  modelName(model: string): string {
    return model
      .split(" · ")
      .map((name) => (Object.hasOwn(models, name) ? models[name as keyof typeof models] : name))
      .join(" · ");
  },
  text(copies: PrivacyCopy[] | undefined, fallback: string): string {
    if (!copies?.length || copies.some((copy) => !Object.hasOwn(sentences, copy.code)))
      return fallback;
    return copies
      .map((copy) => {
        const destination =
          copy.code === "openrouter"
            ? (destinations[copy.destination as keyof typeof destinations] ?? copy.destination)
            : copy.destination;
        return sentences[copy.code as keyof typeof sentences]
          .replace("{material}", () => (copy.material === "text" ? "texto" : (copy.material ?? "")))
          .replace("{destination}", () => destination ?? "");
      })
      .join(" ");
  },
  hubName: "En tu hub",
};
