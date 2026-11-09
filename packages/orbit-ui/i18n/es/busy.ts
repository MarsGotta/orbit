import type { Refusal } from "../../bridge/refusal";

const busyCopy: Record<string, string> = {
  audio: "Espera a que termine la nota de voz.",
  voice_transcription: "Espera a que termine la transcripción de la nota de voz.",
  chat: "Espera a que termine el mensaje en curso.",
  interaction: "Espera a que termine el turno en esta conversación.",
  recording: "Espera a que termine la grabación.",
  call: "Espera a que termine la llamada.",
  calendar: "Espera a que termine la consulta de agenda.",
  playback: "Espera a que termine la reproducción.",
  storage: "Espera a que termine de guardar.",
  ipc_request: "Espera a que termine la petición a Orbit.",
  subscription: "Espera a que termine de conectar la vista.",
  session_list: "Espera a que termine de leer las conversaciones.",
  audio_session: "Espera a que termine de preparar el audio.",
  audio_drain: "Espera a que termine de guardar el audio.",
  capture: "Espera a que termine la captura.",
  computer_action: "Espera a que termine la acción en el Mac.",
  pairing: "Espera a que termine el emparejado.",
  privacy_stop: "Espera a que termine de parar Orbit.",
  files: "Espera a que termine de leer los archivos.",
};

/** Work in progress has a cause and no shareable failure code. */
export function busyText(refusal: Refusal): string {
  return busyCopy[String(refusal.operation)] ?? "Espera a que termine la operación en curso.";
}

export const busyEntry = { class: "working", text: busyText } as const;
