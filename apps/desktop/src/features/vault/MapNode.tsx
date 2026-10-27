import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import { vaultCopy as copy } from "../../i18n/es/vault";
import { COLORS } from "./type-colors";
export function MapNode({
  note,
  x,
  y,
  selected,
  select,
  bright,
}: {
  note: VaultMeta;
  x: number;
  y: number;
  selected: string;
  select: (path: string) => void;
  bright: boolean;
}) {
  return (
    <g
      aria-label={`${note.title} · ${copy.types[note.type as keyof typeof copy.types] ?? note.type} · ${note.links.length + note.backlink_count} enlaces`}
      aria-pressed={selected === note.path}
      data-isolated={note.links.length + note.backlink_count === 0}

      onClick={() => select(note.path)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          select(note.path);
        }
      }}
      opacity={bright ? 1 : 0.3}
      role="button"
      tabIndex={0}
    >
      <circle
        cx={x}
        cy={y}
        fill={COLORS[note.type] ?? COLORS.nota}
        r={Math.min(14, 5 + 0.9 * (note.links.length + note.backlink_count))}
        stroke={note.path === selected ? "var(--o-ink)" : "none"}
        strokeWidth="2"
      />
      <text fill="var(--o-ink2)" fontSize="11" textAnchor="middle" x={x} y={y + 27}>
        {note.title}
      </text>
    </g>
  );
}
