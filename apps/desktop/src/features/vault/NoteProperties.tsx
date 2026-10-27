import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import { noteRoute } from "./route";
import { taskTitle } from "./TaskRows";
import styles from "./VaultReading.module.css";

const LABELS: Record<string, string> = {
  rol: "Rol",
  organizacion: "Organización",
  correo: "Correo",
  estado: "Estado",
  responsable: "Responsable",
  cliente: "Cliente",
  arranque: "Arranque",
  resource: "Origen",
  fecha: "Fecha",
  duracion_min: "Duración (min)",
  serie: "Serie",
  participantes: "Participantes",
  sha256: "SHA-256",
};
const FIELDS: Record<string, string[]> = {
  persona: ["rol", "organizacion", "correo"],
  proyecto: ["estado", "responsable", "cliente", "arranque"],
  fuente: ["resource", "fecha", "duracion_min", "serie", "participantes", "sha256"],
};

export function PropertyValue({ value, notes }: { value: unknown; notes: VaultMeta[] }) {
  if (typeof value === "string") {
    if (value.startsWith("/") && value.endsWith(".md"))
      return <a href={noteRoute(value)}>{taskTitle(value, notes)}</a>;
    return <>{value}</>;
  }
  return <>{value === undefined ? "—" : JSON.stringify(value)}</>;
}

export function NoteProperties({ note, notes }: { note: VaultNote; notes: VaultMeta[] }) {
  const type = typeof note.frontmatter.type === "string" ? note.frontmatter.type : "";
  const projects =
    type === "persona"
      ? notes.filter((entry) => entry.type === "proyecto" && entry.links.includes(note.path))
      : [];
  return (
    <dl className={styles.properties}>
      {(FIELDS[type] ?? []).map((field) => (
        <div key={field}>
          <dt>{LABELS[field]}</dt>
          <dd>
            <PropertyValue notes={notes} value={note.frontmatter[field]} />
          </dd>
        </div>
      ))}
      {projects.length > 0 && (
        <div>
          <dt>Proyectos</dt>
          <dd>
            {projects.map((project, i) => (
              <span key={project.path}>
                {i > 0 && " · "}
                <a href={noteRoute(project.path)}>{project.title}</a>
              </span>
            ))}
          </dd>
        </div>
      )}
      {type === "proyecto" && (
        <div>
          <dt>Equipo</dt>
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
