import type { Refusal } from "../../bridge/refusal";

const busyCopy: Record<string, string> = {
  audio: "Wait for the voice note to finish.",
  voice_transcription: "Wait for the voice note transcription to finish.",
  chat: "Wait for the current message to finish.",
  interaction: "Wait for the turn in this conversation to finish.",
  recording: "Wait for the recording to finish.",
  call: "Wait for the call to finish.",
  calendar: "Wait for the agenda query to finish.",
  playback: "Wait for playback to finish.",
  storage: "Wait for saving to finish.",
  ipc_request: "Wait for the request to Orbit to finish.",
  subscription: "Wait for the view to connect.",
  session_list: "Wait for conversations to finish loading.",
  audio_session: "Wait for audio preparation to finish.",
  audio_drain: "Wait for audio to finish saving.",
  capture: "Wait for capture to finish.",
  computer_action: "Wait for the action on this Mac to finish.",
  pairing: "Wait for pairing to finish.",
  privacy_stop: "Wait for Orbit to finish stopping.",
  files: "Wait for files to finish loading.",
};

/** Work in progress has a cause and no shareable failure code. */
export function busyText(refusal: Refusal): string {
  return busyCopy[String(refusal.operation)] ?? "Wait for the current operation to finish.";
}

export const busyEntry = { class: "working", text: busyText } as const;
