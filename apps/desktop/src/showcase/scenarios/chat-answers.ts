import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { message, SESSION } from "../data/chat";
import { NOW } from "../data/time";
import type { HubData, Scenario } from "../scenario";
import { chat, withChat } from "./chat";

/** The open conversation holds `messages` and nothing else. */
const only =
  (messages: HubData["messages"][string]) =>
  (base: HubData): HubData => ({ ...base, messages: { [SESSION]: messages } });

const CODE_ANSWER = `Aquí lo tienes. Lee las tarifas del hub y guarda una fila por hotel y día:

\`\`\`python exportar_tarifas.py
import csv, sys
from nubaria_hub import tarifas      # cliente del hub

semana = int(sys.argv[sys.argv.index("--semana") + 1])
filas = tarifas(semana=semana)       # hotel, día, precio

with open(f"tarifas-semana-{semana}.csv", "w", newline="") as f:
    escritor = csv.writer(f)
    escritor.writerow(["hotel", "dia", "precio"])
    escritor.writerows(filas)
\`\`\`

Para probarlo, abre Terminal en la carpeta Piloto Ribera y ejecuta:

\`\`\`sh
$ python3 exportar_tarifas.py --semana 43
\`\`\``;

/** A script with its file, and the command to try it. */
const codeConversation = [
  message({
    id: "c1",
    role: "user",
    text: "Hazme un script que exporte las tarifas de la semana a un CSV para Javier.",
    at_ms: NOW - 120_000,
  }),
  message({ id: "c2", role: "assistant", text: CODE_ANSWER, at_ms: NOW - 108_000 }),
];

const deleteQuestion = message({
  id: "c3",
  role: "user",
  text: "Bórrame los CSV de las semanas 41 y 42, ya no hacen falta.",
  at_ms: NOW - 8_000,
});

/** What the hub carries of a command the agent asks to run: the tool's own name. */
const deleteWork: AgentWork = {
  turn_id: "c3",
  started_at_ms: NOW - 8_000,
  tools: 1,
  subjects: ["Bash"],
  items: [{ id: "d1", kind: "tool", phase: "started", label: "Bash", steps: [] }],
};

/** A command that deletes: the hub offers it only once or no, with how many files. */
const deleteApproval: ChatPending = {
  kind: "approval",
  id: "borrar-1",
  text: "rm tarifas-semana-41.csv tarifas-semana-42.csv",
  choices: ["once", "deny"],
  deletes: { paths: 2 },
};

const ANSWER_TOOLS = "role=button[name='Copiar respuesta'] >> nth=-1";

export const chatAnswerScenarios: Scenario[] = [
  chat({
    id: "chat-respuesta-iconos",
    state: "Bajo cada respuesta, iconos",
    description:
      "Copiar, Escuchar, Volver a preguntar y Editar y copiar, solo iconos: el tooltip de «Volver a preguntar» sale encima al pasar el ratón.",
    views: withChat({ work: undefined }),
    ready: ANSWER_TOOLS,
    steps: [{ hover: "role=button[name='Volver a preguntar'] >> nth=-1" }],
  }),
  chat({
    id: "chat-respuesta-copiada",
    state: "Copiar pasa a ✓",
    description: "Tras pulsar Copiar, el icono es un ✓ verde y el tooltip dice «Copiado».",
    views: withChat({ work: undefined }),
    ready: ANSWER_TOOLS,
    steps: [{ click: ANSWER_TOOLS }, { hover: "role=button[name='Copiado']" }],
  }),
  chat({
    id: "chat-codigo-y-comando",
    state: "Código y un comando para ti",
    description:
      "El código con su lenguaje y su archivo, «Python · exportar_tarifas.py», y el comando para ejecutarlo tú: una línea con $ que se copia sin él.",
    size: { width: 1440, height: 1100 },
    views: withChat({ work: undefined }),
    hub: only(codeConversation),
    ready: "role=figure[name='Comando']",
    steps: [{ click: "role=button[name='Copiar comando']" }],
  }),
  chat({
    id: "chat-borrar-permiso",
    state: "Un comando que borra",
    description:
      "Su permiso se da de uno en uno: solo «Permitir una vez» y «No permitir», qué borra y por qué no hay más.",
    size: { width: 1440, height: 1100 },
    views: withChat({
      turn: [deleteQuestion],
      pending: deleteApproval,
      activity: "thinking",
      work: deleteWork,
    }),
    hub: only(codeConversation),
    ready: "text=Un comando que borra solo se permite de uno en uno.",
  }),
];
