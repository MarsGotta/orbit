import type { AppCommand } from "../../bridge/generated";
import type { NoticeCode } from "../../bridge/generated/NoticeCode";
import type { noticeClass } from "../../bridge/generated/noticeClasses";

/** The notice codes Rust classes as a failure the person cannot fix. */
export type FailedNoticeCode = {
  [P in NoticeCode]: (typeof noticeClass)[P] extends "failed" ? P : never;
}[NoticeCode];

/**
 * What could not be done, as it ends «No se pudo …»: one phrase per order,
 * so a refusal says which order failed. The compiler asks for a new order's.
 */
const commands: Record<AppCommand["command"], string> = {
  approval: "mandar tu respuesta",
  ask_orbit_about: "preguntar sobre esta reunión",
  assistant_folder_add: "añadir la carpeta",
  assistant_folder_remove: "quitar la carpeta",
  audio_sources_refresh: "ver qué audio se puede grabar",
  calendar: "guardar el cambio en el calendario",
  calendar_refresh: "actualizar el calendario",
  calendar_set_range: "cargar esas fechas del calendario",
  calendar_show_today: "mostrar hoy en el calendario",
  call_dismiss: "cerrar el aviso de la llamada",
  call_preference: "guardar qué hacer al empezar una videollamada",
  call_record: "grabar la llamada",
  call_start: "empezar la llamada",
  call_stop: "colgar",
  chat: "enviar el mensaje",
  clarify: "enviar tu respuesta",
  error_reports: "cambiar los informes de errores",
  handover: "seguir la conversación con otro asistente",
  interrupt: "detener la respuesta",
  login_item: "cambiar si Orbit se abre al iniciar sesión",
  meeting_ask: "mandar la pregunta",
  meeting_finish: "terminar la grabación",
  meeting_language: "cambiar el idioma",
  meeting_pause: "pausar la grabación",
  meeting_question_close: "cerrar la pregunta",
  meeting_question_hold: "mantener la respuesta en el panel",
  meeting_rename: "cambiar el título",
  meeting_resume: "seguir grabando",
  meeting_start: "empezar a grabar",
  new_session: "abrir una conversación nueva",
  note_action_toggle: "guardar la acción",
  note_edit: "guardar la nota",
  note_library: "guardar la plantilla o el diccionario",
  note_private_save: "guardar tu nota privada",
  note_regenerate: "rehacer el resumen",
  note_restore_superseded: "recuperar la versión anterior de la nota",
  notes_more: "cargar más reuniones",
  notes_open: "abrir la nota",
  notes_refresh: "actualizar tus reuniones",
  onboarding_done: "terminar la bienvenida",
  open_system_privacy: "abrir Ajustes del Sistema",
  open_url: "abrir el enlace",
  pair: "emparejar este Mac",
  permissions: "pedir el permiso",
  privacy_stop: "parar todo",
  profile_set: "guardar tu nombre",
  quit: "salir de Orbit",
  reminder_read: "leer el recordatorio del hub",
  reminder_save: "guardar el recordatorio en el hub",
  reminder_write: "guardar el recordatorio en el hub",
  rearm: "reactivar Orbit",
  recording_keep: "seguir grabando la llamada",
  restart: "reiniciar Orbit",
  session_open: "abrir la conversación",
  speak: "leer la respuesta en voz alta",
  speak_stop: "parar la lectura",
  speaker_rename: "cambiar quién habla",
  transcript_retranscribe: "rehacer la transcripción",
  transcript_retry: "transcribir lo que faltaba",
  update_check: "buscar una versión nueva",
  update_install: "instalar la versión nueva",
  voice_cancel: "cancelar la nota de voz",
  voice_discard: "descartar la nota de voz",
  voice_end: "mandar la nota de voz",
  voice_note_audio: "reproducir la nota de voz",
  voice_retry: "volver a mandar la nota de voz",
  voice_start: "empezar la nota de voz",
};

/** The same, for the daemon's failures; `unknown` is a code this build does not know. */
const notices: Record<FailedNoticeCode, string> = {
  recording_close_unconfirmed: "confirmar el final de la grabación",
  recording_interrupted_by_restart: "seguir la grabación al reiniciarse Orbit",
  capture_refused: "empezar a grabar",
  capture_cut: "seguir grabando",
  recording_seal_failed: "cerrar la grabación; el audio está guardado",
  audio_stop_unconfirmed: "confirmar que el audio se paró",
  audio_start_refused: "empezar el audio",
  mac_command_refused: "hacer en el Mac lo que se pidió",
  mac_command_degraded: "hacer del todo en el Mac lo que se pidió",
  helper_error: "terminar una tarea en el Mac",
  shortcut_refused: "hacer lo que pide el atajo",
  voice_note_refused: "mandar la nota de voz",
  assistant_failed: "terminar la respuesta",
  assistant_interrupt_unconfirmed: "confirmar que la respuesta se detuvo",
  approval_outcome_unknown: "saber si se aplicó tu respuesta",
  call_stopped_by_connection: "mantener la llamada",
  call_drain_unconfirmed: "confirmar el final de la llamada",
  call_detection_failed: "detectar la videollamada",
  hub_error: "terminar lo que se pidió al hub",
  hub_device_identity_missing: "confirmar este Mac con el hub",
  stop_step_failed: "parar todo del todo",
  contained_failure: "terminar una tarea",
  meeting_question_not_sent: "mandar la pregunta",
  speech_failed: "leer la respuesta en voz alta",
  store_problem: "guardar los datos en este Mac",
  closed_with_pending_work: "terminar de guardar antes de cerrar",
  hub_event_failed: "procesar un mensaje del hub",
  work_failed: "terminar una tarea",
  store_recovery_lost_files: "recuperar todos los archivos",
  source_failed: "leer una fuente de audio",
  note_change_refused: "guardar el cambio en la nota",
  upload_refused: "subir el audio al hub; se queda en este Mac",
  unknown: "completar la acción",
};

/**
 * The generic failure (design guide 3, «Fallos»): what could not be done,
 * what to do, and the code to share. No «administrador»: the person runs
 * their own Orbit, and whoever runs their hub reads the code in its log.
 */
const helpBefore = "Vuelve a intentarlo. Si se repite, comparte el código ";
const helpAfter = " con quien gestiona tu hub.";

export const failures = {
  failure: {
    title: (what: string) => `No se pudo ${what}`,
    help: (code: string) => `${helpBefore}${code}${helpAfter}`,
    // The help around its code, which the notice sets apart to be read aloud.
    helpBefore,
    helpAfter,
    fallback: "completar la acción",
    renderFailed: "mostrar esta vista",
    retry: "Reintentar",
    copyCode: "Copiar el código",
    copying: "Copiando…",
    copied: "✓ Copiado",
    commands,
    notices,
  },
} as const;
