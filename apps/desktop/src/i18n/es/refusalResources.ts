const resources = {
  subscriptions: "suscripciones",
  ipc_response: "respuesta local",
  ipc_request: "petición local",
  hub_response: "respuesta del hub",
  message: "mensaje",
  recording: "grabación",
  session: "conversación",
  audio: "audio",
  calendar: "calendario",
  source: "fuente",
  capture: "captura",
  computer_action: "acción en el Mac",
  storage: "almacenamiento",
  folder: "carpeta",
  file: "archivo",
};
export type RefusalResource = keyof typeof resources;

export function refusalResource(resource: unknown): string {
  const code = String(resource);
  return Object.hasOwn(resources, code) ? resources[code as RefusalResource] : code;
}
