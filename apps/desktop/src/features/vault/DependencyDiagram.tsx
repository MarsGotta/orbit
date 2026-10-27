import { DependencyNode } from "./DependencyNode";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import styles from "./VaultReading.module.css";

function layout(note: VaultNote) {
  const ids = [
    ...new Set(note.dependencies.flatMap((edge) => [edge.prerequisite, edge.dependent])),
  ];
  const ranks = new Map(ids.map((id) => [id, 0]));
  for (let pass = 0; pass < ids.length; pass += 1) {
    let changed = false;
    for (const edge of note.dependencies) {
      const rank = Math.min(ids.length, (ranks.get(edge.prerequisite) ?? 0) + 1);
      if (rank > (ranks.get(edge.dependent) ?? 0)) {
        ranks.set(edge.dependent, rank);
        changed = true;
      }
    }
    if (!changed) break;
  }
  const rows = new Map<number, number>();
  return ids.map((id) => {
    const rank = ranks.get(id) ?? 0;
    const row = rows.get(rank) ?? 0;
    rows.set(rank, row + 1);
    const task = note.tasks.find((task) => `${task.file}#${task.block_id}` === id);
    const label =
      task?.text ??
      (id.endsWith("#milestone-arranque")
        ? "Arranque del piloto"
        : `Referencia: ${id.split("#").pop() ?? id}`);
    const blocked = note.dependencies.some(
      (edge) =>
        edge.dependent === id &&
        note.tasks.find((t) => `${t.file}#${t.block_id}` === edge.prerequisite)?.done !== true,
    );
    return {
      id,
      label,
      x: rank * 210 + 10,
      y: row * 108 + 10,
      state:
        task?.done === true
          ? "done"
          : id.endsWith("#milestone-arranque")
            ? "milestone"
            : blocked
              ? "blocked"
              : "pending",
    };
  });
}

export function DependencyDiagram({ note }: { note: VaultNote }) {
  const nodes = layout(note);
  if (nodes.length === 0) return <p>No hay dependencias declaradas en esta nota.</p>;
  const width = Math.max(...nodes.map((node) => node.x)) + 190;
  const height = Math.max(...nodes.map((node) => node.y)) + 100;
  return (
    <div className={styles.dependency}>
      <p className={styles.blockLabel}>Dependencias · se leen de las tareas y sus enlaces</p>
      <div className={styles.diagramScroll}>
        <svg
          aria-label="Diagrama de dependencias"
          height={height}
          role="img"
          viewBox={`0 0 ${width} ${height}`}
          width={width}
        >
          <Arrowhead />
          {note.dependencies.map((edge, i) => {
            const from = nodes.find((node) => node.id === edge.prerequisite),
              to = nodes.find((node) => node.id === edge.dependent);
            return from === undefined || to === undefined ? null : (
              <line
                key={i}
                markerEnd="url(#vault-arrow)"
                stroke="var(--o-line2)"
                x1={from.x + 174}
                x2={to.x}
                y1={from.y + 37}
                y2={to.y + 37}
              />
            );
          })}
          {nodes.map((node) => (
            <DependencyNode key={node.id} node={node} />
          ))}
        </svg>
      </div>
      <details>
        <summary>Dependencias en texto</summary>
        <ul>
          {note.dependencies.map((edge, i) => (
            <li key={i}>
              {nodes.find((node) => node.id === edge.dependent)?.label} depende de{" "}
              {nodes.find((node) => node.id === edge.prerequisite)?.label}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

function Arrowhead() {
  return (
    <defs>
      <marker id="vault-arrow" markerHeight="6" markerWidth="6" orient="auto" refX="5" refY="3">
        <path d="M0,0 L6,3 L0,6" fill="var(--o-line2)" />
      </marker>
    </defs>
  );
}
