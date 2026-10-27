import type { VaultNote } from "../../bridge/generated/VaultNote";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { displayDate } from "./TaskRows";
import styles from "./VaultReading.module.css";

function actor(value: unknown): string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return "Sin datos";
  const event = value as { by?: unknown; at?: unknown };
  return `${typeof event.by === "string" ? event.by : "Sin datos"}${typeof event.at === "string" ? ` · ${displayDate(event.at)}` : ""}`;
}

export function NoteFacts({ note }: { note: VaultNote }) {
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
          <dd>{typeof fields.type === "string" ? fields.type : "Sin type"}</dd>
        </div>
        <div>
          <dt>Nota</dt>
          <dd>
            {fields.status === "draft"
              ? "Borrador"
              : fields.status === "deprecated"
                ? "Retirada"
                : "Estable"}
          </dd>
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
            {Array.isArray(fields.tags)
              ? fields.tags.map((tag) =>
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
