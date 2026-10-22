import { describe, expect, it } from "vitest";
import { folderSkill, skillName, utf8Bytes } from "./skill-file";

// A skill is named by its SKILL.md's frontmatter; a folder gives its
// top-level SKILL.md. Invented names.

function file(path: string, name = path.split("/").at(-1) ?? path): File {
  const made = new File(["x"], name);
  Object.defineProperty(made, "webkitRelativePath", { value: path });
  return made;
}

describe("the name of a SKILL.md", () => {
  it.each([
    ["---\nname: notas-de-viaje\ndescription: x\n---\n", "notas-de-viaje"],
    ['\uFEFF---\r\nname: "notas"\r\n---\r\n', "notas"],
    ["---\ndescription: x\nname: 'citas'\n---\nname: cuerpo", "citas"],
  ])("reads %j as %s", (content, name) => {
    expect(skillName(content)).toBe(name);
  });

  it.each([
    ["", "nothing"],
    ["name: suelta\n", "no frontmatter"],
    ["---\ndescription: x\n---\nname: cuerpo\n", "a name only in the body"],
    ["---\nname: \ndescription: x\n---\n", "an empty name"],
  ])("has none in %j (%s)", (content) => {
    expect(skillName(content)).toBeNull();
  });
});

describe("the SKILL.md of a chosen folder", () => {
  it("is the one at its top, not one deeper inside", () => {
    const top = file("viaje/SKILL.md");
    expect(folderSkill([file("viaje/scripts/SKILL.md"), top, file("viaje/notas.md")])).toEqual({
      folder: "viaje",
      file: top,
    });
  });

  it("is missing when the folder has none at its top", () => {
    expect(folderSkill([file("viaje/scripts/SKILL.md")])).toEqual({ folder: "viaje", file: null });
  });

  it("is the file itself when the panel chose a SKILL.md, not a folder", () => {
    const chosen = file("", "SKILL.md");
    expect(folderSkill([chosen])).toEqual({ folder: "SKILL.md", file: chosen });
  });
});

it("measures UTF-8 bytes as the hub does", () => {
  expect(utf8Bytes("Sé")).toBe(3);
});
