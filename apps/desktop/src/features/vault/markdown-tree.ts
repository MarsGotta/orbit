import type { VaultNote } from "../../bridge/generated/VaultNote";

interface Node {
  type: string;
  value?: string;
  url?: string;
  lang?: string;
  identifier?: string;
  checked?: boolean;
  children?: Node[];
  data?: { hProperties: Record<string, string> };
  position?: { start: { line: number }; end: { line: number } };
}

function inline(value: string, note: VaultNote): Node[] {
  const nodes: Node[] = [];
  let end = 0;
  for (const match of value.matchAll(/\[\[([^\]\n]+)\]\]|\[\^([^\]\n]+)\]/g)) {
    nodes.push({ type: "text", value: value.slice(end, match.index) });
    const raw = match[1];
    if (raw !== undefined) {
      const [target = "", alias] = raw.split("|");
      const link = note.outlinks.findIndex((link) => link.target === target);
      nodes.push(
        link < 0
          ? { type: "text", value: match[0] }
          : {
              type: "link",
              url: `#orbit-note-${link}`,
              children: [{ type: "text", value: alias ?? target }],
            },
      );
    } else {
      const citation = note.citations.find((c) => c.id === match[2]);
      nodes.push(
        citation === undefined
          ? { type: "text", value: match[0] }
          : sourceNode(citation.id, citation.number),
      );
    }
    end = match.index + match[0].length;
  }
  nodes.push({ type: "text", value: value.slice(end) });
  return nodes;
}
function sourceNode(id: string, number: number): Node {
  return {
    type: "link",
    url: `#orbit-source-${encodeURIComponent(id)}`,
    children: [{ type: "text", value: String(number) }],
  };
}

/** Transform text nodes, never fenced/inline code or existing links. */
export function remarkVault(note: VaultNote) {
  return () => (root: Node) => {
    const walk = (node: Node) => {
      if (node.type === "link" || node.type === "code" || node.type === "inlineCode") return;
      node.children = node.children?.flatMap((child) => {
        if (child.type === "footnoteDefinition") return [];
        if (child.type === "footnoteReference") {
          const citation = note.citations.find((c) => c.id.toLowerCase() === child.identifier);
          return citation === undefined ? [] : [sourceNode(citation.id, citation.number)];
        }
        if (child.type === "text") return inline(child.value ?? "", note);
        if (
          child.type === "list" &&
          child.children?.every((item) => typeof item.checked === "boolean") === true
        ) {
          const offset =
            note.raw.slice(0, note.raw.length - note.body.length).split("\n").length - 1;
          return [
            {
              type: "code",
              lang: "orbit-tasks",
              value: "",
              data: {
                hProperties: {
                  "data-orbit-task-range": JSON.stringify({
                    from: (child.position?.start.line ?? 1) + offset,
                    to: (child.position?.end.line ?? 1) + offset,
                  }),
                },
              },
            },
          ];
        }
        walk(child);
        return [child];
      });
    };
    walk(root);
  };
}

export function internalTarget(source: string, href: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("//") || href.includes("\\"))
    return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(href.split("#")[0] ?? "");
  } catch {
    return null;
  }
  if (!decoded.toLowerCase().endsWith(".md")) return null;
  const parts = decoded.startsWith("/") ? [] : source.split("/").slice(0, -1);
  for (const part of decoded.split("/")) {
    if (part === "..") {
      if (parts.length === 0) return null;
      parts.pop();
    } else if (part !== "" && part !== ".") parts.push(part);
  }
  return parts.join("/");
}
