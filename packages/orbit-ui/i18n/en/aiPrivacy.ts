import type { PrivacyCopy } from "../../bridge/generated/PrivacyCopy";

const sentences = {
  openrouter: "The {material} is sent to {destination}.",
  network:
    "The {material} is sent to {destination}, outside your hub, over your network or the Internet.",
  hub: "The {material} stays on your hub.",
  operator: "The operator configuration is used; the service may send data outside the hub.",
  vault_legacy: "Notes model saved by the operator; the hub job does not run yet.",
  unconfigured: "No operator model is configured for this function.",
  hermes:
    "The {material} is sent to Hermes at {destination}; its configuration determines whether data goes to another provider.",
};

const destinations = {
  microsoft_openrouter: "Microsoft through OpenRouter",
  openrouter_provider: "OpenRouter and the model provider",
};

const models = {
  "Hermes listening": "Hermes listening",
  "Hermes speech": "Hermes speech",
  "No model configured": "No model configured",
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
          .replace("{material}", () => copy.material ?? "")
          .replace("{destination}", () => destination ?? "");
      })
      .join(" ");
  },
  hubName: "On your hub",
};
