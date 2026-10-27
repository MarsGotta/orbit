export const transcription = {
  title: "Transcripción",
  label: "Modelo de transcripción",
  whisper: "Whisper en tu hub",
  mai: "MAI-Transcribe-2 (OpenRouter)",
  privacy:
    "El audio de reuniones, notas de voz y llamadas se envía a Microsoft a través de OpenRouter. Se solicita retención cero de datos (ZDR).",
  scope: "Esta elección se guarda en el hub y se aplica a todos sus Mac.",
  key: "Clave de OpenRouter",
  keyStored: "Ya hay una clave guardada en el hub. Deja el campo vacío para conservarla.",
  keyHelp: "Se valida antes de guardar y se conserva cifrada en el hub. La clave no vuelve al Mac.",
  save: "Guardar transcripción",
  saving: "Validando y guardando…",
  saved: "Guardado en el hub",
  loading: "Leyendo la transcripción del hub…",
  unavailable: "Conecta con el hub para cambiar la transcripción.",
  invalid: "El hub no devolvió la configuración de transcripción.",
  compare: "Volver a transcribir con…",
} as const;

export const transcriptionRefusals: Record<string, string> = {
  transcription_key_missing: "Introduce la clave de OpenRouter en Ajustes › Reuniones.",
  transcription_key_invalid: "La clave de OpenRouter no es válida.",
  transcription_key_forbidden: "La clave de OpenRouter no tiene acceso a MAI.",
  transcription_quota_exhausted: "Se ha agotado el saldo de OpenRouter.",
  transcription_rate_limited: "OpenRouter ha limitado las solicitudes; reinténtalo.",
  transcription_connection_error: "No se pudo conectar con el servicio de transcripción.",
  transcription_timeout: "El servicio de transcripción tardó demasiado.",
};
