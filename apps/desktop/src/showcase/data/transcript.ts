import type { NoteSpeakerSnapshot } from "../../bridge/generated/NoteSpeakerSnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import { MINUTE } from "./time";

export const speakers: NoteSpeakerSnapshot[] = [
  {
    key: "mic_local",
    track: "mic_local",
    name: "Lucía Ferrer",
    color: "var(--o-c1)",
    share_pct: 34,
  },
  {
    key: "remote_1",
    track: "remote_app",
    name: "Marta Ibáñez",
    color: "var(--o-c4)",
    share_pct: 38,
  },
  {
    key: "remote_2",
    track: "remote_app",
    name: "Diego Salinas",
    color: "var(--o-c2)",
    share_pct: 17,
  },
  {
    key: "remote_3",
    track: "remote_app",
    name: "Javier Ortega",
    color: "var(--o-c3)",
    share_pct: 11,
  },
];

const lines: Array<[number, string, string]> = [
  [
    0.2,
    "mic_local",
    "Vale, empezamos. Hoy quiero cerrar el flujo de reserva para el piloto de Ribera.",
  ],
  [
    0.6,
    "remote_1",
    "Traigo dos versiones del paso de pago. La primera mantiene los tres pasos; la segunda lo junta todo en una pantalla.",
  ],
  [1.4, "remote_2", "¿Cuál funcionó mejor en las pruebas con recepcionistas?"],
  [
    1.7,
    "remote_1",
    "La de una pantalla. Tardaron casi la mitad y nadie se perdió buscando el botón de confirmar.",
  ],
  [2.5, "mic_local", "Entonces vamos con esa. ¿Qué nos falta para tenerla el lunes?"],
  [
    2.9,
    "remote_3",
    "Por mi parte, que la disponibilidad cargue rápido. Ahora tarda unos dos segundos con cinco hoteles.",
  ],
  [
    3.6,
    "remote_2",
    "Eso con cincuenta hoteles no aguanta. Habría que cachear las tarifas por día.",
  ],
  [4.2, "remote_3", "Lo puedo tener el jueves si no tocamos el modelo de habitaciones."],
  [4.8, "mic_local", "No lo tocamos. Marta, ¿el calendario semanal está listo?"],
  [
    5.1,
    "remote_1",
    "Falta el estado vacío, cuando un hotel no tiene habitaciones libres esa semana.",
  ],
  [5.9, "mic_local", "Que diga qué semana es la siguiente con hueco, en vez de dejarlo en blanco."],
  [6.4, "remote_1", "Me gusta. Lo añado al prototipo y lo enseño en la daily."],
  [7.0, "remote_2", "Otra cosa: el cliente pidió poder bloquear habitaciones para grupos."],
  [7.5, "mic_local", "Eso lo dejamos para después del piloto. Lo apunto como tema abierto."],
  [8.2, "remote_3", "Perfecto. Entonces yo con la caché y Marta con el estado vacío."],
  [8.6, "mic_local", "Y yo hablo con Pablo para confirmar la fecha de arranque. Gracias a todos."],
];

export const turns: NoteTurnSnapshot[] = lines.map(([minutes, speaker_key, text], index) => ({
  index,
  at_ms: Math.round(minutes * MINUTE),
  speaker_key,
  text,
}));
