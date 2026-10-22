// Creating, moving, editing and deleting events from the calendar.
// Kept beside `agenda.ts`, which merges it into `es.calendar`.

const readOnlyCalendar = "Este calendario es de solo lectura";
const unnamed = "Orbit no puede cambiar este evento";

export const agendaEdit = {
  write: {
    saving: "Guardando…",
    deleted: "Evento borrado",
    deleting: (title: string) => `Borrando «${title}»…`,
    /** `lost`: what does not come back, already joined («sin sus 3 invitados y …»). */
    undoLoses: (lost: string) => `Deshacer lo crea de nuevo ${lost}.`,
    and: "y",
    withoutGuests: (count: number) =>
      count === 1 ? "sin su invitado" : `sin sus ${count} invitados`,
    withoutReminders: (count: number) =>
      count === 1
        ? "sin 1 recordatorio que Orbit no edita"
        : `sin ${count} recordatorios que Orbit no edita`,
    cannotUndo: (title: string) => `El último cambio de «${title}» no se puede deshacer.`,
    outsideSeries: "como evento suelto, fuera de la serie",
    undoing: (title: string) => `Deshaciendo el último cambio de «${title}»…`,
    nothingToUndo: "No hay ningún cambio de esta sesión que deshacer.",
    // What the user can do about it; the cause stays in the log.
    unknown: "No se sabe si el cambio se guardó. Compruébalo en el calendario.",
    refused: "El calendario no aplicó el cambio. No se cambió nada.",
    // The same, for a write that ended while the calendar view was closed.
    unseenRefused: "Un cambio del calendario no se guardó",
    unseenUnknown: "No se sabe si un cambio del calendario se guardó",
    openCalendar: "Abrir el calendario",
    dismiss: "Cerrar aviso",
    blocked: {
      organizer: {
        move: "No puedes mover un evento que organiza otra persona",
        edit: "No puedes cambiar un evento que organiza otra persona",
        delete: "No puedes borrar un evento que organiza otra persona",
      },
      read_only: { move: readOnlyCalendar, edit: readOnlyCalendar, delete: readOnlyCalendar },
      unnamed: { move: unnamed, edit: unnamed, delete: unnamed },
    },
  },
  edit: {
    newEvent: "Nuevo evento",
    title: "Título",
    when: "Cuándo",
    startDate: "Fecha de inicio",
    startTime: "Hora de inicio",
    endDate: "Fecha de fin",
    endTime: "Hora de fin",
    repeat: "Repetición",
    repeatOptions: {
      none: "No se repite",
      daily: "Cada día",
      weekly: "Cada semana",
      monthly: "Cada mes",
      yearly: "Cada año",
      kept: "Regla propia del Mac",
    },
    every: "Cada",
    interval: "Intervalo de repetición",
    units: {
      daily: (count: number) => (count === 1 ? "día" : "días"),
      weekly: (count: number) => (count === 1 ? "semana" : "semanas"),
      monthly: (count: number) => (count === 1 ? "mes" : "meses"),
      yearly: (count: number) => (count === 1 ? "año" : "años"),
    },
    locationPlaceholder: "Añadir ubicación",
    notes: "Notas del evento",
    notesPlaceholder: "Añadir notas",
    notesTooLong: "Las notas son demasiado largas para Orbit: edítalas en Calendario.",
    calendar: "Calendario",
    reminders: "Avisarme",
    reminderOptions: [
      [0, "Al empezar"],
      [5, "5 min"],
      [10, "10 min"],
      [15, "15 min"],
      [30, "30 min"],
      [60, "1 h"],
      [1440, "1 día"],
    ] as const,
    minutesBefore: (minutes: number) => `${minutes} min`,
    remindersKept: (count: number) =>
      count === 1
        ? "Tiene además 1 recordatorio que Orbit no edita; se conserva."
        : `Tiene además ${count} recordatorios que Orbit no edita; se conservan.`,
    remindersTooMany: "Tiene más recordatorios de los que Orbit edita: cámbialos en Calendario.",
    availability: "Mostrarme como",
    busy: "Ocupada",
    free: "Libre",
    create: "Crear evento",
    save: "Guardar",
    saveThis: "Solo este",
    saveFuture: "Este y los siguientes",
    delete: "Borrar",
    deleteThis: "Solo este",
    deleteFuture: "Este y los siguientes",
    needsTitle: "El evento necesita un título",
    noChanges: "Sin cambios",
    saved: "✓ Guardado en tu calendario",
    changeWhen: "Cambiar fecha y hora",
    readOnly: {
      organizer: (name: string | null) =>
        `Lo organiza ${name ?? "otra persona"}: solo quien lo organiza puede cambiarlo.`,
      read_only: () =>
        `${readOnlyCalendar}. Puedes unirte y grabar; los cambios se hacen en Calendario.`,
      unnamed: () => `${unnamed}.`,
    },
    resizeEnd: "Cambiar la hora de fin",
    resizeFirstDay: "Cambiar el primer día",
    resizeLastDay: "Cambiar el último día",
  },
} as const;
