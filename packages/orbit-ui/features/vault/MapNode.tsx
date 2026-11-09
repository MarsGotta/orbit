import { useCopy } from "../../i18n/CopyProvider";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import { COLORS, typeLabel } from "./type-colors";
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
  const copy = useCopy().vaultReading;
  return (
    <g
      aria-label={copy.mapNode(
        note.title,
        typeLabel(note.type, note.role),
        note.links.length + note.backlink_count,
      )}
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
        fill={COLORS[note.role] ?? COLORS.note}
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
