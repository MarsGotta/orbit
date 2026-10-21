export const briefing = {
  briefing: {
    morning: "Buenos días",
    afternoon: "Buenas tardes",
    night: "Buenas noches",
    // Without the Mac's zone the hour of the day is not known.
    hello: "Hola",
    weekPrefix: "W",
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
    agendaTopics: "A tratar",
    addContext: "Añade contexto para esta reunión…",
    localContext: "Tu contexto",
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
    week: "Ver semana",
    noEvents: "Hoy no tienes reuniones",
    noUpcoming: "No quedan reuniones hoy",
  },
} as const;
