import { useCopy } from "../../i18n/CopyProvider";
import type { VaultNote } from "../../bridge/generated/VaultNote";

import { displayDate } from "./TaskRows";
import styles from "./VaultReading.module.css";

function actor(value: unknown): string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return "Sin datos";
  const event = value as { by?: unknown; at?: unknown };
  return `${typeof event.by === "string" ? event.by : "Sin datos"}${typeof event.at === "string" ? ` · ${displayDate(event.at)}` : ""}`;
}

function statusLabel(value: unknown) {
  const labels: Record<string, string> = {
    draft: "Borrador",
    deprecated: "Retirada",
    stable: "Estable",
  };
  return typeof value === "string" ? (labels[value] ?? value) : "Sin estado";
}

export function NoteFacts({ note }: { note: VaultNote }) {
  const copy = useCopy().extras.vaultCopy;
  const fields = note.frontmatter;
  const verified = Array.isArray(fields.verified)
    ? fields.verified
    : fields.verified === undefined
      ? []
      : [fields.verified];
  return (
    <>
      <h3>{copy.props}</h3>
      <dl className={styles.properties}>
        <div>
          <dt>Tipo</dt>
          <dd>{typeof fields.type === "string" ? fields.type : note.type}</dd>
        </div>
        <div>
          <dt>Nota</dt>
          <dd>{statusLabel(note.properties.status)}</dd>
        </div>
        <div>
          <dt>Escrita por</dt>
          <dd>{actor(fields.generated)}</dd>
        </div>
        <div>
          <dt>Revisión</dt>
          <dd>{verified.length === 0 ? "Sin revisar" : verified.map(actor).join("; ")}</dd>
        </div>
        <div>
          <dt>Caduca</dt>
          <dd>
            {typeof fields.stale_after === "string"
              ? `Revisar antes del ${displayDate(fields.stale_after)}`
              : "Sin fecha"}
          </dd>
        </div>
        <div>
          <dt>Etiquetas</dt>
          <dd>
            {Array.isArray(note.properties.tags)
              ? note.properties.tags.map((tag) =>
                  typeof tag === "string" ? (
                    <span className={styles.tag} key={tag}>
                      {tag}
                    </span>
                  ) : null,
                )
              : "Sin etiquetas"}
          </dd>
        </div>
      </dl>
    </>
  );
}
