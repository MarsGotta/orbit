import type { ActivityCopy } from "../../bridge/generated/ActivityCopy";

import type { ActivityLabelCode } from "../../bridge/generated/ActivityLabelCode";

const labels: Record<Exclude<ActivityLabelCode, "unknown">, string> = {
  click_app: "Pulsando en {value}",
  click: "Pulsando en la pantalla",
  type: "Escribiendo",
  keys: "Pulsando teclas",
  scroll_app: "Desplazándose por {value}",
  scroll: "Desplazándose",
  open_app: "Abriendo {value}",
  open: "Abriendo una app",
  action_tool: "Usando {value} en el Mac",
  action: "Actuando en el Mac",
  command_value: "Ejecutando: {value}",
  command: "Ejecutando una orden",
  edit_value: "Editando: {value}",
  edit: "Editando archivos",
  screenshot_app: "Mirando {value}",
  screenshot: "Mirando la pantalla",
  plan: "Plan · {done} de {total} pasos",
  orbit_devices: "Mirando los Mac de Orbit",
  orbit_windows: "Mirando las ventanas",
  orbit_look: "Mirando la pantalla",
  orbit_selected_text: "Leyendo el texto seleccionado",
  orbit_control_tools: "Viendo qué puede hacer en el Mac",
  orbit_action_result: "Comprobando cómo acabó la acción",
  orbit_calendar: "Leyendo el calendario",
  orbit_notes_search: "Buscando en las notas",
  orbit_note_read: "Leyendo una nota",
  orbit_notes_index: "Leyendo el índice de notas",
  orbit_meetings: "Leyendo las reuniones",
  orbit_transcript: "Leyendo la transcripción de una reunión",
  orbit_note: "Leyendo la nota de una reunión",
  orbit_files_list: "Mirando las carpetas del Mac",
  orbit_files_search: "Buscando en las carpetas del Mac",
  orbit_files_read: "Leyendo un archivo del Mac",
  orbit_attachment: "Leyendo un adjunto del mensaje",
  orbit_skill: "Cargando una de sus skills",
};

const subjects = {
  hub_terminal: "la terminal del hub",
  hub_files: "archivos del hub",
  macs: "los Mac",
  windows: "las ventanas",
  screen: "la pantalla",
  selected_text: "el texto seleccionado",
  mac: "el Mac",
  calendar: "el calendario",
  notes: "las notas",
  note: "una nota",
  notes_index: "el índice de notas",
  meetings: "las reuniones",
  folders: "las carpetas",
  attachments: "los adjuntos",
  skills: "sus skills",
};

export const agentActivity = {
  labels,
  subjects,
  label(copy: ActivityCopy | undefined, fallback: string): string {
    if (!copy || !Object.hasOwn(labels, copy.code)) return fallback;
    const values = {
      value: copy.value ?? "",
      done: String(copy.done ?? 0),
      total: String(copy.total ?? 0),
    };
    return labels[copy.code as keyof typeof labels].replace(
      /\{(value|done|total)\}/g,
      (_, key: keyof typeof values) => values[key],
    );
  },
  subject(code: string | undefined, fallback: string): string {
    return code && Object.hasOwn(subjects, code)
      ? subjects[code as keyof typeof subjects]
      : fallback;
  },
};
