export const briefing = {
  briefing: {
    morning: "Buenos días",
    afternoon: "Buenas tardes",
    night: "Buenas noches",
    // Without the Mac's zone the hour of the day is not known.
    hello: "Hola",
    weekNumber: (week: number) => `semana ${week}`,
    summaryMeetings: "reuniones",
    summaryMeeting: "reunión",
    zeroMeetings: "0 reuniones",
    focus: "de foco",
    startsIn: "Empieza en",
    inProgress: "En curso",
    nextPrefix: "Siguiente:",
    minutes: "min",
    recording: "Grabando",
    previousTitle: (title: string) => `Nota anterior: ${title}`,
    // What the previous note of the series left open.
    agendaTopics: "Pendiente de la última vez",
    // Kept in this Mac only: it never reaches the hub or the note.
    personalNote: "Recordatorio personal",
    personalNotePlaceholder: "Algo que no quieras olvidar…",
    personalNoteHelp: "Solo lo ves tú, en este Mac. No va a la nota.",
    joinRecord: "Unirse y grabar",
    finish: "Terminar",
    previousNote: "Abrir nota anterior",
    // The agenda not read yet says nothing; without the permission
    // it says how to grant it.
    calendarPermission: "Concede el permiso de Calendarios para ver tu agenda.",
    timezoneFailed: "No se pudo leer la zona horaria del Mac.",
    peopleCount: (count: number) =>
      count === 0 ? "solo" : count === 1 ? "1 persona" : `${count} personas`,
    agenda: "Agenda de hoy",
    week: "Ver la semana",
    noEvents: "Hoy no tienes reuniones",
    noUpcoming: "No quedan reuniones hoy",
  },
} as const;
