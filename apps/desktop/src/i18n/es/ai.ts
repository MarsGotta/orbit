export const aiCopy = {
  title: "IA",
  lead: "Tus proveedores y el modelo que hace cada cosa. Las claves se guardan cifradas en el hub.",
  functions: {
    meeting_transcription: "Transcripción de reuniones",
    listening: "Notas de voz y llamadas: escuchar",
    speech: "Voz de los asistentes: hablar",
    meeting_notes: "Resumen y nota de la reunión",
    vault: "Destilado y revisión de Notas",
    assistants: "Asistentes compatibles con OpenAI",
  } as Record<string, string>,
  capabilities: {
    text: "Texto",
    tools: "Herramientas",
    transcription: "Transcripción",
    word_timestamps: "Tiempos por palabra",
    speech: "Voz",
  } as Record<string, string>,
  unavailable: "No se pudo leer la configuración de IA. Comprueba la conexión con tu hub.",
  saved: "Guardado en el hub",
  providerName: "Nombre del proveedor",
  baseUrl: "URL base",
  key: "Clave (opcional)",
  keyHelp:
    "Déjala vacía para conservar la clave guardada en la misma URL. Nunca vuelve desde el hub.",
  add: "Añadir un proveedor",
  save: "Comprobar y guardar",
  saving: "Comprobando en el hub…",
  local: "IA en local",
  cloud: "En la nube, con clave",
  operator: "Configuración del operador",
};

const aiRefusals: Record<string, string> = {
  ai_key_invalid: "El proveedor rechazó la clave",
  ai_key_missing: "Introduce la clave de OpenRouter",
  ai_provider_unreachable: "No se pudo conectar con el proveedor; comprueba su URL",
  ai_invalid_catalog: "El proveedor devolvió un catálogo de modelos no válido",
  ai_settings_invalid:
    "Revisa el nombre, la URL, los modelos y la clave del proveedor. La URL requiere HTTPS, salvo en loopback, y no admite credenciales, consulta ni fragmento",
  ai_settings_busy: "Ya hay una comprobación de IA en curso; espera a que termine",
  ai_provider_invalid:
    "Revisa el identificador del proveedor y su URL; la URL de OpenRouter no se puede cambiar",
  ai_provider_missing: "El proveedor elegido no está configurado",
  ai_capability_missing: "El modelo no tiene la capacidad que necesita esta función",
  ai_function_invalid: "La función de IA no existe",
  ai_models_invalid: "Corrige sólo los modelos que devolvió el proveedor",
};

export function aiRefusalText(
  refusal: import("../../bridge/generated/AgentRefusal").AgentRefusal,
): string | undefined {
  const n = refusal.numbers;
  switch (refusal.code) {
    case "ai_provider_timeout":
      return `El proveedor no terminó la comprobación en ${String(n.timeout_ms)} ms.`;
    case "ai_provider_http_error":
      return `El proveedor devolvió HTTP ${String(n.provider_status)}; comprueba su saldo, cuota o disponibilidad.`;
    case "ai_catalog_size":
      return `El catálogo tiene ${String(n.count)} modelos; debe tener entre 1 y ${String(n.max_count)}.`;
    case "ai_providers_full":
      return `Hay ${String(n.count)} proveedores; el máximo es ${String(n.max_count)}.`;
    case "ai_response_too_large":
    case "ai_registry_full":
      return `La configuración o respuesta ocupa ${String(n.bytes)} bytes; el máximo es ${String(n.max_bytes)}.`;
    default:
      return Object.hasOwn(aiRefusals, refusal.code)
        ? `${aiRefusals[refusal.code]}${n.provider_status === undefined ? "" : ` (HTTP ${String(n.provider_status)})`}.`
        : undefined;
  }
}
