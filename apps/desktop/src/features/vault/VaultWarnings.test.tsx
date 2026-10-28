import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CopyProvider } from "../../i18n/CopyProvider";
import { configureCopy } from "../../i18n";
import { VaultWarnings } from "./VaultWarnings";

afterEach(() => configureCopy("es", [], "Deimos"));

describe.each(["es", "en"] as const)("vault warning copy in %s", (language) => {
  it("renders the schema cause and line from its diagnostic", () => {
    render(
      <CopyProvider choice={language}>
        <VaultWarnings
          warnings={["UNTRUSTED"]}
          diagnostics={[{ code: "schema_types_mapping", field: "types", numbers: { line: 2 } }]}
        />
      </CopyProvider>,
    );
    expect(screen.getByRole("alert").textContent).toBe(
      language === "es"
        ? "SCHEMA.md: types debe ser un mapa (línea 2). Se usan los valores por defecto."
        : "SCHEMA.md: types must be a mapping (line 2). Defaults are used.",
    );
  });

  it("names unknown and older warnings without displaying backend prose", () => {
    const view = render(
      <CopyProvider choice={language}>
        <VaultWarnings
          warnings={["UNTRUSTED"]}
          diagnostics={[{ code: "future_warning", field: "private field", numbers: {} }]}
        />
      </CopyProvider>,
    );
    expect(screen.getByRole("alert").textContent).toContain("future_warning");
    expect(screen.getByRole("alert").textContent).not.toContain("UNTRUSTED");
    view.rerender(
      <CopyProvider choice={language}>
        <VaultWarnings warnings={["UNTRUSTED"]} />
      </CopyProvider>,
    );
    expect(screen.getByRole("alert").textContent).toContain("legacy_vault_warning");
  });
});
