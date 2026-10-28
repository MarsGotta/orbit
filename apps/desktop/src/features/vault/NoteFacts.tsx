import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { VaultNote } from "../../bridge/generated/VaultNote";

import { displayDate } from "./TaskRows";
import styles from "./VaultReading.module.css";

function actor(value: unknown): string {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return getCopy().vaultReading.noData;
  const event = value as { by?: unknown; at?: unknown };
  return `${typeof event.by === "string" ? event.by : getCopy().vaultReading.noData}${typeof event.at === "string" ? ` · ${displayDate(event.at)}` : ""}`;
}

function statusLabel(value: unknown) {
  const labels: Record<string, string> = {
    draft: getCopy().vaultReading.draft,
    deprecated: getCopy().vaultReading.deprecated,
    stable: getCopy().vaultReading.stable,
  };
  return typeof value === "string" ? (labels[value] ?? value) : getCopy().vaultReading.noStatus;
}

export function NoteFacts({ note }: { note: VaultNote }) {
  const inlineCopy = useCopy().vaultReading;
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
          <dt>{inlineCopy.type}</dt>
          <dd>{typeof fields.type === "string" ? fields.type : note.type}</dd>
        </div>
        <div>
          <dt>{inlineCopy.note}</dt>
          <dd>{statusLabel(note.properties.status)}</dd>
        </div>
        <div>
          <dt>{inlineCopy.writtenBy}</dt>
          <dd>{actor(fields.generated)}</dd>
        </div>
        <div>
          <dt>{inlineCopy.review}</dt>
          <dd>{verified.length === 0 ? inlineCopy.unreviewed : verified.map(actor).join("; ")}</dd>
        </div>
        <div>
          <dt>{inlineCopy.expires}</dt>
          <dd>
            {typeof fields.stale_after === "string"
              ? inlineCopy.reviewBefore(displayDate(fields.stale_after))
              : inlineCopy.noDate}
          </dd>
        </div>
        <div>
          <dt>{inlineCopy.tags}</dt>
          <dd>
            {Array.isArray(note.properties.tags)
              ? note.properties.tags.map((tag) =>
                  typeof tag === "string" ? (
                    <span className={styles.tag} key={tag}>
                      {tag}
                    </span>
                  ) : null,
                )
              : inlineCopy.noTags}
          </dd>
        </div>
      </dl>
    </>
  );
}
