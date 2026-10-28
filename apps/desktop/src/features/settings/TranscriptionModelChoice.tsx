import { useCopy } from "../../i18n/CopyProvider";
import type { TranscriptionModel } from "../../bridge/generated/TranscriptionModel";

export function TranscriptionModelChoice({
  model,
  onChange,
}: {
  model?: TranscriptionModel;
  onChange: (model?: TranscriptionModel) => void;
}) {
  const inlineCopy = useCopy().settingsInline;
  const modelCopy = useCopy().extras.transcription;
  return (
    <>
      <label>
        {modelCopy.compare}
        <select
          aria-label={modelCopy.compare}
          value={model ?? ""}
          onChange={(event) => onChange((event.target.value as TranscriptionModel) || undefined)}
        >
          <option value="">{inlineCopy.hubModel}</option>
          <option value="whisper">{modelCopy.whisper}</option>
          <option value="mai">{modelCopy.mai}</option>
        </select>
      </label>
      {model === "mai" && <span>{modelCopy.privacy}</span>}
    </>
  );
}
