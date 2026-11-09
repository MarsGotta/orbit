/** Bytes of `text` as UTF-8, as the hub measures its ceilings. */
export function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

/**
 * The `name` of a `SKILL.md`'s frontmatter: the skill's name, and so its
 * route on the hub. `null` when there is no frontmatter or no name; the hub
 * judges whether the name is valid.
 */
export function skillName(content: string): string | null {
  const lines = content.replace(/^\uFEFF/, "").split(/\r?\n/);
  if (lines[0]?.trim() !== "---") return null;
  for (const line of lines.slice(1)) {
    if (line.trim() === "---") return null;
    const match = /^name:\s*(.*?)\s*$/.exec(line);
    if (match) {
      const name = (match[1] ?? "").replace(/^(["'])(.*)\1$/, "$2").trim();
      return name === "" ? null : name;
    }
  }
  return null;
}

/**
 * The `SKILL.md` at the top of a folder chosen in the panel, by the paths
 * WebKit gives each file (`carpeta/SKILL.md`); a `SKILL.md` deeper inside is
 * not the folder's. The panel also lets a file be chosen: a `SKILL.md`
 * chosen alone has no folder path and is taken as it is.
 */
export function folderSkill(files: readonly File[]): { folder: string; file: File | null } {
  const alone = files.length === 1 && files[0]?.webkitRelativePath === "" ? files[0] : undefined;
  if (alone) return { folder: alone.name, file: alone.name === "SKILL.md" ? alone : null };
  const folder = files[0]?.webkitRelativePath.split("/")[0] ?? "";
  const file =
    files.find((candidate) => {
      const parts = candidate.webkitRelativePath.split("/");
      return parts.length === 2 && parts[1] === "SKILL.md";
    }) ?? null;
  return { folder, file };
}
