export interface DiagramNode {
  id: string;
  label: string;
  x: number;
  y: number;
  state: string;
}
export function DependencyNode({ node }: { node: DiagramNode }) {
  return (
    <g data-state={node.state}>
      <title>
        {node.label} ·{" "}
        {node.state === "done" ? "Hecha" : node.state === "blocked" ? "Bloqueada" : "Pendiente"}
      </title>
      <rect
        fill="var(--o-surface)"
        height="75"
        rx="8"
        stroke={node.state === "milestone" ? "var(--o-accText)" : "var(--o-line2)"}
        strokeDasharray={node.state === "blocked" ? "5 4" : undefined}
        width="174"
        x={node.x}
        y={node.y}
      />
      <circle
        cx={node.x + 13}
        cy={node.y + 19}
        fill={
          node.state === "done"
            ? "var(--o-ok)"
            : node.state === "milestone"
              ? "var(--o-accText)"
              : "var(--o-ink3)"
        }
        r="3"
      />
      <text fill="var(--o-ink)" fontSize="11" x={node.x + 23} y={node.y + 23}>
        {node.label.slice(0, 23)}
        <tspan x={node.x + 13} y={node.y + 43}>
          {node.label.slice(23, 49)}
        </tspan>
        <tspan fill="var(--o-ink3)" x={node.x + 13} y={node.y + 63}>
          {node.state === "done" ? "Hecha" : node.state === "blocked" ? "Bloqueada" : "Pendiente"}
        </tspan>
      </text>
    </g>
  );
}
