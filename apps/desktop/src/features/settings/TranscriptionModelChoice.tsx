import type { TranscriptionModel } from "../../bridge/generated/TranscriptionModel";
import { transcription as modelCopy } from "../../i18n/es/transcription";

export function TranscriptionModelChoice({
  model,
  onChange,
}: {
  model?: TranscriptionModel;
  onChange: (model?: TranscriptionModel) => void;
}) {
  return (
    <>
      <label>
        {modelCopy.compare}
        <select
          aria-label={modelCopy.compare}
          value={model ?? ""}
          onChange={(event) => onChange((event.target.value as TranscriptionModel) || undefined)}
        >
          <option value="">Modelo del hub</option>
          <option value="whisper">{modelCopy.whisper}</option>
          <option value="mai">{modelCopy.mai}</option>
        </select>
      </label>
      {model === "mai" && <span>{modelCopy.privacy}</span>}
    </>
  );
}
