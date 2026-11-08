const causes: Record<string, string> = {
  stale_revision: "Otra revisión cambió la propuesta. Vuelve a leer antes de continuar.",
  distillation_edit_invalid:
    "La edición no cumple el tipo de propiedad, las citas o los enlaces permitidos.",
  preparation_interrupted: "La preparación se interrumpió al reiniciar el hub.",
  invalid_model_shape: "El modelo devolvió una propuesta incompleta o con formato no válido.",
  incomplete_completion: "El modelo alcanzó su límite de salida antes de terminar.",
  quote_not_in_transcript: "Una cita del modelo no coincide con la transcripción.",
  patch_quote_missing: "Falta la cita que justifica un cambio.",
  invalid_proposal: "La propuesta no cumple el formato de las notas.",
  distillation_internal_error: "El hub no pudo terminar la preparación por un error interno.",
  vault_io_failed: "El hub no pudo leer los archivos del vault.",
  write_interrupted: "La escritura se interrumpió antes de confirmarse.",
  distillation_write_failed:
    "Falló la escritura de las notas; consulta el estado antes de continuar.",
  distillation_content_changed: "Una nota cambió desde la preparación; vuelve a preparar.",
  distillation_revision_changed:
    "Otra revisión cambió la propuesta. Vuelve a leer antes de continuar.",
  distillation_revision_conflict:
    "Otra revisión cambió la propuesta. Vuelve a leer antes de continuar.",
  distillation_undecided: "Falta decidir una contradicción.",
  distillation_not_found: "La propuesta ya no existe; puede haberse borrado la reunión.",
  distillation_vault_changed: "La propuesta pertenece a otra conexión de Notas.",
  distillation_too_large: "La propuesta supera el tamaño máximo admitido.",
  distillation_state: "La propuesta no admite esta acción en su estado actual.",
  schema_owner_invalid: "El identificador owner de SCHEMA.md no es válido.",
  ambiguous_person_name: "Hay más de una ficha para la misma persona.",
  new_link_target_missing: "Un enlace nuevo no tiene una nota de destino.",
  source_is_immutable: "La propuesta intenta cambiar una fuente que debe conservarse.",
  invalid_source_metadata: "La fuente no cumple el formato requerido.",
  overlapping_patches: "La propuesta tiene cambios que se solapan.",
  patch_context_not_unique: "No se puede localizar de forma única el texto que cambia.",
  exactly_one_meeting_note_required: "La propuesta debe contener una ficha de la reunión.",
  invalid_task_date: "Una tarea tiene una fecha no válida.",
};
export function distillationFailure(code: string, numbers: Record<string, unknown>) {
  const label = causes[code] ?? `La propuesta no se puede completar (${code}).`;
  return `${label}${Object.entries(numbers)
    .map(([key, value]) => ` · ${key}: ${String(value)}`)
    .join("")}`;
}
export function distillationCause(code: string) {
  return causes[code];
}
