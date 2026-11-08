const causes: Record<string, string> = {
  stale_revision: "Another review changed the proposal. Read it again before continuing.",
  distillation_edit_invalid: "The edit does not follow the allowed property type, quotes or links.",
  preparation_interrupted: "Preparation was interrupted when the hub restarted.",
  invalid_model_shape: "The model returned an incomplete or malformed proposal.",
  incomplete_completion: "The model reached its output limit before finishing.",
  quote_not_in_transcript: "A model quote does not match the transcript.",
  patch_quote_missing: "A change is missing its supporting quote.",
  invalid_proposal: "The proposal does not follow the notes format.",
  distillation_internal_error: "An internal hub error prevented preparation from finishing.",
  vault_io_failed: "The hub could not read the vault files.",
  write_interrupted: "Writing was interrupted before confirmation.",
  distillation_write_failed: "Writing the notes failed; read their status before continuing.",
  distillation_content_changed: "A note changed after preparation; prepare again.",
  distillation_revision_changed:
    "Another review changed the proposal. Read it again before continuing.",
  distillation_revision_conflict:
    "Another review changed the proposal. Read it again before continuing.",
  distillation_undecided: "A contradiction still needs a decision.",
  distillation_not_found: "The proposal no longer exists; its meeting may have been deleted.",
  distillation_vault_changed: "The proposal belongs to another Notes connection.",
  distillation_too_large: "The proposal exceeds the maximum supported size.",
  distillation_state: "This action is not available in the proposal’s current state.",
  schema_owner_invalid: "The owner id in SCHEMA.md is invalid.",
  ambiguous_person_name: "More than one note matches the same person.",
  new_link_target_missing: "A new link has no target note.",
  source_is_immutable: "The proposal tries to change a source that must be preserved.",
  invalid_source_metadata: "The source does not follow the required format.",
  overlapping_patches: "The proposal has overlapping changes.",
  patch_context_not_unique: "The text to change cannot be located uniquely.",
  exactly_one_meeting_note_required: "The proposal must contain one meeting note.",
  invalid_task_date: "A task has an invalid date.",
};
export function distillationFailure(code: string, numbers: Record<string, unknown>) {
  const label = causes[code] ?? `The proposal cannot be completed (${code}).`;
  return `${label}${Object.entries(numbers)
    .map(([key, value]) => ` · ${key}: ${String(value)}`)
    .join("")}`;
}
export function distillationCause(code: string) {
  return causes[code];
}
