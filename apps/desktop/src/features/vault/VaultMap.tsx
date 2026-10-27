import { COLORS } from "./type-colors";
import { MapNode } from "./MapNode";
import { useState } from "react";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { noteRoute } from "./route";
import styles from "./VaultReading.module.css";

function isolated(note: VaultMeta) {
  return note.links.length + note.backlink_count === 0;
}
function positions(notes: VaultMeta[]) {
  const linked = notes.filter((note) => !isolated(note));
  const loose = notes.filter(isolated);
  const top = linked.length > 0 ? 580 : 40;
  return [
    ...linked.map((note, i) => ({
      note,
      x: 440 + Math.cos((i * 2 * Math.PI) / linked.length) * (i % 2 === 0 ? 270 : 340),
      y: 290 + Math.sin((i * 2 * Math.PI) / linked.length) * (i % 2 === 0 ? 190 : 240),
    })),
    ...loose.map((note, i) => ({
      note,
      x: 150 + (i % 3) * 290,
      y: top + 40 + Math.floor(i / 3) * 55,
    })),
  ];
}
function LooseLabel({ nodes }: { nodes: ReturnType<typeof positions> }) {
  const first = nodes.find(({ note }) => isolated(note));
  return first === undefined ? null : (
    <text fill="var(--o-ink3)" fontSize="12" x="20" y={first.y - 28}>
      Notas sueltas
    </text>
  );
}

function Graph({
  notes,
  selected,
  select,
}: {
  notes: VaultMeta[];
  selected: string;
  select: (path: string) => void;
}) {
  const nodes = positions(notes);
  const edges = nodes.flatMap((node) =>
    node.note.links.flatMap((path) => {
      const target = nodes.find((n) => n.note.path === path);
      return target === undefined ? [] : [{ from: node, to: target }];
    }),
  );
  const neighbors = new Set(
    edges
      .filter((edge) => [edge.from.note.path, edge.to.note.path].includes(selected))
      .flatMap((edge) => [edge.from.note.path, edge.to.note.path]),
  );
  return (
    <svg
      aria-label="Mapa de enlaces entre notas"
      className={styles.graph}
      role="group"
      viewBox={`0 0 880 ${Math.max(610, ...nodes.map((node) => node.y + 55))}`}
    >
      <LooseLabel nodes={nodes} />
      {edges.map(({ from, to }, i) => (
        <line
          key={i}
          opacity={selected === "" || [from.note.path, to.note.path].includes(selected) ? 1 : 0.15}
          stroke={
            selected !== "" && [from.note.path, to.note.path].includes(selected)
              ? "var(--o-accText)"
              : "var(--o-line2)"
          }
          x1={from.x}
          x2={to.x}
          y1={from.y}
          y2={to.y}
        />
      ))}
      {nodes.map(({ note, x, y }) => (
        <MapNode
          key={note.path}
          note={note}
          x={x}
          y={y}
          selected={selected}
          select={select}
          bright={selected === "" || note.path === selected || neighbors.has(note.path)}
        />
      ))}
    </svg>
  );
}

export function VaultMap({ notes }: { notes: VaultMeta[] }) {
  const [types, setTypes] = useState(Object.keys(copy.types).filter((type) => type !== "fuente"));
  const [selected, select] = useState("");
  const filtered = notes.filter((note) => types.includes(note.type));
  const picked = filtered.find((note) => note.path === selected);
  return (
    <section className={styles.content}>
      <header className={styles.heading}>
        <h1>{copy.map}</h1>
        <p>{copy.mapLead}</p>
      </header>
      <div className={styles.filters}>
        {Object.entries(copy.types).map(([type, label]) => (
          <button
            aria-pressed={types.includes(type)}
            key={type}
            onClick={() =>
              setTypes(types.includes(type) ? types.filter((t) => t !== type) : [...types, type])
            }
            type="button"
          >
            <span aria-hidden="true" style={{ color: COLORS[type] }}>
              ●{" "}
            </span>
            {label}
          </button>
        ))}
      </div>
      <div className={styles.mapArea}>
        <Graph notes={filtered} select={select} selected={selected} />
        {picked !== undefined && (
          <div className={styles.mapCard}>
            <small>{copy.types[picked.type as keyof typeof copy.types] ?? picked.type}</small>
            <h2>{picked.title}</h2>
            <p>{picked.description}</p>
            <p>
              {picked.links.length + picked.backlink_count} enlaces
              {picked.links.length + picked.backlink_count === 0 ? " · nota suelta" : ""}
            </p>
            <a href={noteRoute(picked.path)}>Abrir la nota</a>
          </div>
        )}
      </div>
      <details>
        <summary>Lista equivalente del mapa · {filtered.length} notas</summary>
        <ul>
          {filtered.map((note) => (
            <li key={note.path}>
              <a href={noteRoute(note.path)}>{note.title}</a> ·{" "}
              {note.links.length + note.backlink_count} enlaces
            </li>
          ))}
        </ul>
      </details>
    </section>
  );
}
