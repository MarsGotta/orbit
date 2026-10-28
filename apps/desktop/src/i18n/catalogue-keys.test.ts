import { afterEach, expect, it } from "vitest";
import { configureCopy, getCopy } from ".";
import { settingsSectionKey, SETTINGS_SECTIONS } from "../state/ui";

afterEach(() => configureCopy("es", ["es-ES"], "Atlas"));

it.each(["es", "en"] as const)(
  "uses English catalogue keys without changing route labels in %s",
  (language) => {
    configureCopy(language, [language], "Atlas");
    const copy = getCopy();
    expect(
      SETTINGS_SECTIONS.map((section) => copy.shell.settingsSections[settingsSectionKey(section)]),
    ).toEqual(
      language === "es"
        ? [
            "General",
            "Privacidad y permisos",
            "Grabación y llamadas",
            "Asistentes",
            "IA",
            "Reuniones",
            "Notas",
          ]
        : [
            "General",
            "Privacy & Permissions",
            "Recording & Calls",
            "Assistants",
            "AI",
            "Meetings",
            "Notes",
          ],
    );
    expect(copy.extras.assistantsCopy.primaryBadge).toBe(language === "es" ? "Principal" : "Main");
    expect(copy.extras.assistantsCopy.primary).toBe(
      language === "es" ? "Asistente principal" : "Primary assistant",
    );
    const spanish = new Set([
      "privacidad",
      "grabacion",
      "asistentes",
      "reuniones",
      "notas",
      "ia",
      "principal",
    ]);
    function keys(value: unknown): string[] {
      if (typeof value !== "object" || value === null) return [];
      return Object.entries(value).flatMap(([key, child]) => [key, ...keys(child)]);
    }
    expect(keys(copy).filter((key) => spanish.has(key))).toEqual([]);
  },
);
