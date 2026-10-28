import { afterEach, expect, it } from "vitest";
import { configureCopy, getCopy } from ".";

afterEach(() => configureCopy("es", ["es-ES"], "Atlas"));

it.each([
  [
    "es",
    ["0 tareas abiertas", "1 tarea abierta", "2 tareas abiertas"],
    ["0 enlaces", "1 enlace", "2 enlaces"],
    ["0 notas · 0 fuentes", "1 nota · 1 fuente", "2 notas · 2 fuentes"],
  ],
  [
    "en",
    ["0 open tasks", "1 open task", "2 open tasks"],
    ["0 links", "1 link", "2 links"],
    ["0 notes · 0 sources", "1 note · 1 source", "2 notes · 2 sources"],
  ],
] as const)(
  "inflects every vault count in %s for zero, one and two",
  (language, tasks, links, totals) => {
    configureCopy(language, [language], "Atlas");
    const copy = getCopy().vaultReading;
    for (const count of [0, 1, 2]) {
      expect(copy.taskCount(count)).toBe(tasks[count]);
      expect(copy.linkCount(count)).toBe(links[count]);
      expect(copy.counts(count, count)).toBe(totals[count]);
      expect(copy.mapNode("Example", "project", count)).toBe(`Example · project · ${links[count]}`);
      expect(copy.mapList(count)).toContain(
        `${count} ${language === "es" ? (count === 1 ? "nota" : "notas") : count === 1 ? "note" : "notes"}`,
      );
      expect(copy.pageCounts(count, count)).toContain(
        `${count} ${language === "es" ? (count === 1 ? "nota" : "notas") : count === 1 ? "note" : "notes"}`,
      );
      expect(copy.vaultCounts(count, count, "Example")).toContain(
        language === "es"
          ? count === 1
            ? "1 nota y 1 fuente"
            : `${count} notas y ${count} fuentes`
          : count === 1
            ? "1 note and 1 source"
            : `${count} notes and ${count} sources`,
      );
    }
  },
);
