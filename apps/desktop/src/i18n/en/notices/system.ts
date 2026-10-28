import { numberFormat } from "../../locale";
import { fileSize } from "../attachments";
import { openSettings, rearm, worded, type Entries, NoticeData } from "./types";

/** A folder chosen in the panel that Orbit did not keep, with its cause and numbers. */
function folderNotAdded(data: NoticeData): string {
  const { code, cause, count, max, bytes, max_bytes: maxBytes, status } = data;
  if (code === "too_many" && typeof max === "number") {
    const asked =
      typeof count === "number" ? ` (would be folder ${numberFormat().format(count)})` : "";
    return `Folder not added: there are already ${numberFormat().format(max)} folders, the maximum${asked}. Remove one to add another.`;
  }
  if (cause === "path_too_long" && typeof bytes === "number" && typeof maxBytes === "number")
    return `Folder not added: its path takes ${numberFormat().format(bytes)} bytes; Orbit allows ${numberFormat().format(maxBytes)}.`;
  if (cause === "bookmark_too_large" && typeof bytes === "number" && typeof maxBytes === "number")
    return `Folder not added: its macOS bookmark takes ${numberFormat().format(bytes)} bytes; Orbit stores up to ${numberFormat().format(maxBytes)}.`;
  if (code === "storage_failed")
    return "Folder not added: Orbit couldn’t save it on this Mac. Add it again.";
  if (code === "assistant_folder_panel_busy") return "The folder picker is already open.";
  const why = [
    code,
    cause,
    typeof status === "number" ? `error ${numberFormat().format(status)}` : undefined,
  ]
    .filter((part) => part !== undefined)
    .join(", ");
  return `Couldn’t add the folder: the macOS picker didn’t return it${why ? ` (${why})` : ""}. Try again.`;
}

/**
 * The hub's storage, nearly full or full: how much it uses of its limit,
 * what is left, what to do (free room, or raise the hub's quota when that
 * sets the limit) and what happens if nobody does. The hub deletes nothing.
 */
function hubStorage(data: NoticeData): string {
  const { bytes: used, max_bytes: max, remaining: left, cause } = data;
  const known = typeof used === "number" && typeof max === "number" && typeof left === "number";
  const state =
    known && left === 0
      ? `The hub is full: ${fileSize(used)} of ${fileSize(max)} used.`
      : known
        ? `The hub is almost full: ${fileSize(used)} of ${fileSize(max)} used, with ${fileSize(left)} free.`
        : "The hub is almost full.";
  const fix =
    cause === "quota"
      ? "Free up server space or raise storage_quota_bytes in the hub configuration."
      : "Free up space on the server’s disk.";
  return `${state} ${fix} If it fills up, new meetings stay on this Mac without transcription until space is available. The hub never deletes anything on its own.`;
}

/** Privacy, the hub link and pairing, the local store, transcripts and archives. */
export const system = {
  engine_exited: {
    class: "fixable",
    text: (data) => {
      const process =
        data.cause === "proxy" ? "The control engine’s connection process" : "The control engine";
      const status =
        typeof data.status !== "number"
          ? "without reporting an exit status"
          : data.code === "signal"
            ? `signal ${numberFormat().format(data.status)}`
            : data.code === "exit"
              ? `code ${numberFormat().format(data.status)}`
              : `wait status ${numberFormat().format(data.status)}`;
      return `${process} exited without Orbit requesting it (${status}). Orbit will restart it. No input action will be repeated.`;
    },
  },
  privacy_stopped: worded("done", "Everything is stopped.", rearm),
  privacy_stop_kept: worded("fixable", "Everything is still stopped, as you left it.", rearm),
  privacy_stop_active: worded("fixable", "Everything is stopped. Rearm Orbit to continue.", rearm),
  privacy_rearmed: worded("done", "✓ Orbit is working again"),
  device_revoked: worded(
    "fixable",
    "The hub revoked this Mac. Pair it again in Settings.",
    openSettings,
  ),
  device_pairing_mismatch: worded(
    "fixable",
    "Pairing doesn’t match the hub. Pair again in Settings.",
    openSettings,
  ),
  hub_storage_nearly_full: { class: "fixable", text: hubStorage },
  orbit_closing: worded("working", "Closing Orbit…"),
  pending_work_keep_open: worded("fixable", "Orbit is still saving. Don’t close it yet."),
  pairing_failed: worded("fixable", "Couldn’t pair. Check the code and hub address."),
  paired: worded("done", "Mac paired."),
  conversations_need_hub: worded(
    "fixable",
    "Conversations are on the hub, which is disconnected. Check your connection.",
  ),
  transcript_retranscribe_unconfirmed: {
    class: "fixable",
    text: (data) => {
      let cause = "couldn’t connect to the hub";
      if (data.cause === "hub_response_too_large")
        cause = "the hub response is larger than Orbit can read";
      if (data.cause === "hub_response_cut") cause = "the hub response was interrupted";
      if (data.cause === "hub_response_invalid")
        cause = "the hub response has an unexpected format";
      if (data.cause === "hub_status") cause = "the hub is unavailable right now";
      return `The hub hasn’t confirmed the new transcript: ${cause}. It may still be running.`;
    },
  },
  transcript_retranscribe_refused: {
    class: "fixable",
    text: (data) => {
      if (data.code === "recording_still_open")
        return "Can’t redo the transcript: the recording hasn’t closed yet.";
      if (data.code === "diarization_not_configured")
        return "Can’t redo the transcript: speaker separation isn’t configured on the hub.";
      return `The hub refused to redo the transcript.${data.code ? ` (${data.code})` : ""}`;
    },
  },
  transcript_retry_requested: worded("done", "Retrying transcription."),
  note_change_unsent: worded("fixable", "Change not saved: no connection to the hub."),
  meeting_delete_needs_hub_update: worded("fixable", "Update the hub to delete meetings"),
  turn_no_longer_exists: {
    class: "fixable",
    text: (data) => {
      const at =
        typeof data.at_ms === "number"
          ? ` (${numberFormat().format(Math.floor(data.at_ms / 60_000))}:${String(Math.floor(data.at_ms / 1_000) % 60).padStart(2, "0")})`
          : "";
      const turns =
        typeof data.turns === "number"
          ? ` and now has ${numberFormat().format(data.turns)} turns`
          : "";
      return `That turn${at} no longer exists: the transcript changed since you opened it${turns}. Open it again.`;
    },
  },
  assistant_folder_not_added: { class: "fixable", text: folderNotAdded },
  meeting_audio_unreadable: {
    class: "fixable",
    text: (data) =>
      typeof data.count === "number" && typeof data.total === "number"
        ? `Can’t redo the transcript: audio from ${numberFormat().format(data.count)} of ${numberFormat().format(data.total)} groups can’t be read.`
        : "Can’t redo the transcript: some audio can’t be read.",
  },
} satisfies Partial<Entries>;
