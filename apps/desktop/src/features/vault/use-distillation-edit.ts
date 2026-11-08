import { useState } from "react";
import type { DistillationChange } from "orbit-ui/bridge/generated/DistillationChange";
import type { DistillationReview } from "orbit-ui/bridge/generated/DistillationReview";
function draftOf(change: DistillationChange) {
  return {
    properties: Object.fromEntries(
      change.properties.map((prop) => [
        prop.key,
        typeof prop.after === "string" ? prop.after : (JSON.stringify(prop.after) ?? "null"),
      ]),
    ),
    lines: Object.fromEntries(change.lines.map((line, i) => [String(i), line.after])),
  };
}
export function useDistillationEdit(
  change: DistillationChange,
  revision: number,
  onEditing: (editing: boolean) => void,
  review: (review: DistillationReview) => Promise<boolean>,
) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(() => draftOf(change));
  const [error, setError] = useState(false);
  const start = (value: boolean) => {
    setDraft(draftOf(change));
    setEditing(value);
    onEditing(value);
    setError(false);
  };
  const save = async () => {
    let properties: Record<string, unknown>;
    try {
      properties = Object.fromEntries(
        change.properties.map((prop) => [
          prop.key,
          typeof prop.after === "string"
            ? draft.properties[prop.key]
            : (JSON.parse(draft.properties[prop.key] ?? "null") as unknown),
        ]),
      );
    } catch {
      setError(true);
      return;
    }
    if (await review({ revision, changes: { [change.id]: { properties, lines: draft.lines } } })) {
      setEditing(false);
      onEditing(false);
    }
  };
  const update = (kind: "properties" | "lines", key: string, value: string) =>
    setDraft((before) => ({ ...before, [kind]: { ...before[kind], [key]: value } }));
  return { editing, draft, error, start, save, update };
}
export type DistillationEditor = ReturnType<typeof useDistillationEdit>;
