import { numberFormat } from "../../locale";
import { fileSize } from "../attachments";
import { openSettings, type Entries, NoticeData } from "./types";

/** Bytes as the hub's other storage notices say them: «10 GB». */
function size(bytes: unknown): string | undefined {
  return typeof bytes === "number" ? fileSize(bytes) : undefined;
}

/** «HTTP 409, recording_finalized»: the status and the hub's own code. */
function answer(data: NoticeData): string {
  const status =
    typeof data.status === "number" ? `HTTP ${numberFormat().format(data.status)}` : "no status";
  return typeof data.code === "string" ? `${status}, ${data.code}` : status;
}

function what(data: NoticeData): string {
  return data.step === "recording_finalize" ? "the end of a meeting" : "an audio chunk";
}

/** Uploads to the hub: what it refused or could not store, and what to do. */
export const delivery = {
  upload_refused_retrying: {
    class: "fixable",
    text: (data) =>
      data.status === 404
        ? `The hub couldn’t find where to receive ${what(data)} (${answer(data)}). Retries automatically; audio stays on the Mac. If it continues, check the hub address in Settings.`
        : `The hub doesn’t accept this Mac (${answer(data)}). Pair again in Settings. Audio stays on the Mac and uploads afterward.`,
    action: openSettings,
  },
  hub_storage_full: {
    class: "fixable",
    text: (data) => {
      const used = size(data.bytes);
      const max = size(data.max_bytes);
      const free = size(data.remaining);
      const numbers = [
        used && max ? `using ${used} of ${max}` : used ? `using ${used}` : undefined,
        free ? `${free} free` : undefined,
      ]
        .filter((part) => part !== undefined)
        .join(", ");
      return `The hub has no space for more audio${numbers ? ` (${numbers})` : ""}. Audio stays on the Mac and uploads when space is available. Free up hub storage or raise its limit.`;
    },
  },
} satisfies Partial<Entries>;

/** A refusal the hub will repeat for the same bytes: said, never retried. */
export const uploadRefused: Entries<"upload_refused"> = {
  upload_refused: {
    class: "failed",
    text: (data) =>
      `The hub rejected ${what(data)} (${answer(data)}). It stays on the Mac without retrying; the meeting closes without it.`,
  },
};
