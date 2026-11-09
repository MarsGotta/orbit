import { useCopy } from "../../i18n/CopyProvider";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import { noteRoute } from "./route";
import { taskTitle } from "./TaskRows";
import styles from "./VaultReading.module.css";

const METADATA = [
  "status",
  "type",
  "title",
  "description",
  "tags",
  "sources",
  "generated",
  "verified",
  "stale_after",
];

export function PropertyValue({ value, notes }: { value: unknown; notes: VaultMeta[] }) {
  if (typeof value === "string") {
    if (value.startsWith("/") && value.endsWith(".md"))
      return <a href={noteRoute(value)}>{taskTitle(value, notes)}</a>;
    return <>{value}</>;
  }
  return <>{value === undefined ? "—" : JSON.stringify(value)}</>;
}

export function NoteProperties({
  note,
  notes,
  fieldNames,
}: {
  note: VaultNote;
  notes: VaultMeta[];
  fieldNames: Partial<Record<string, string>>;
}) {
  const inlineCopy = useCopy().vaultReading;
  const labels: Record<string, string> = {
    stage: inlineCopy.projectStage,
    start: inlineCopy.projectStart,
  };
  const projects =
    note.role === "person"
      ? notes.filter((entry) => entry.role === "project" && entry.links.includes(note.path))
      : [];
  const fields = {
    ...Object.fromEntries(
      Object.entries(note.frontmatter).filter(
        ([key]) => !METADATA.includes(key) && !Object.values(fieldNames).includes(key),
      ),
    ),
    ...Object.fromEntries(Object.entries(note.properties).filter(([key]) => key in labels)),
  };
  return (
    <dl className={styles.properties}>
      {Object.entries(fields).map(([field, value]) => (
        <div key={field}>
          <dt>{labels[field] ?? field}</dt>
          <dd>
            <PropertyValue notes={notes} value={value} />
          </dd>
        </div>
      ))}
      <ProjectLinks projects={projects} />
      {note.role === "project" && (
        <div>
          <dt>{inlineCopy.team}</dt>
          <dd className={styles.team}>
            {[...new Set(note.tasks.map((task) => task.who).filter((who) => who !== ""))].map(
              (who) => (
                <span className={styles.avatar} key={who} title={who}>
                  {who.charAt(0)}
                </span>
              ),
            )}
          </dd>
        </div>
      )}
    </dl>
  );
}

function ProjectLinks({ projects }: { projects: VaultMeta[] }) {
  const inlineCopy = useCopy().vaultReading;
  if (projects.length === 0) return null;
  return (
    <div>
      <dt>{inlineCopy.projects}</dt>
      <dd>
        {projects.map((project, index) => (
          <span key={project.path}>
            {index > 0 && " · "}
            <a href={noteRoute(project.path)}>{project.title}</a>
          </span>
        ))}
      </dd>
    </div>
  );
}
