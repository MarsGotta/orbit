import { expect, it } from "vitest";
import { resolveLanguage } from ".";

it("uses the first supported system language, or English", () => {
  expect(resolveLanguage("system", ["es-ES"])).toEqual({ language: "es", locale: "es-ES" });
  expect(resolveLanguage("system", ["en-GB"])).toEqual({ language: "en", locale: "en-GB" });
  expect(resolveLanguage("system", ["fr-FR"])).toEqual({ language: "en", locale: "en-US" });
  expect(resolveLanguage("system", ["fr-FR", "es-MX", "en-GB"]).language).toBe("es");
  expect(resolveLanguage("system", ["en-US", "es-ES"]).language).toBe("en");
  expect(resolveLanguage("system", []).language).toBe("en");
  expect(resolveLanguage("en", ["es-ES", "en-GB"]).locale).toBe("en-GB");
  expect(resolveLanguage("es", ["en-GB"]).locale).toBe("es-ES");
  expect(resolveLanguage("en", ["es-ES"]).locale).toBe("en-US");
});
