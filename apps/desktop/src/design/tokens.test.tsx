import { expect, it } from "vitest";
import fontCss from "./fonts.css?inline";
import tokenCss from "./tokens.css?inline";

it("does not ask Google for fonts from the webview", () => {
  expect(`${fontCss}\n${tokenCss}`).not.toMatch(/fonts\.(?:googleapis|gstatic)\.com/);
});

/** The custom properties a block of the stylesheet defines, by the selector that opens it. */
function definedIn(opening: string): Map<string, string> {
  const start = tokenCss.indexOf(opening);
  expect(start, opening).toBeGreaterThanOrEqual(0);
  const body = tokenCss.slice(start + opening.length, tokenCss.indexOf("}", start));
  const names = new Map<string, string>();
  for (const [, name, value] of body.matchAll(/(--o-[\w-]+):\s*([^;]+);/g))
    if (name && value) names.set(name, value.trim());
  return names;
}

it("the dark theme, the system's light one and the fixed light one define the same names", () => {
  const dark = definedIn(":root {");
  const systemLight = definedIn(':root:not([data-theme="dark"]) {');
  const fixedLight = definedIn(':root[data-theme="light"] {');
  expect([...systemLight.keys()].sort()).toEqual([...dark.keys()].sort());
  expect(fixedLight).toEqual(systemLight);
});

it("has the selection and the word under the mouse, in both themes", () => {
  const dark = definedIn(":root {");
  const light = definedIn(':root[data-theme="light"] {');
  expect(dark.get("--o-sel")).toBe("rgba(139, 124, 246, 0.35)");
  expect(dark.get("--o-wordHover")).toBe("rgba(139, 124, 246, 0.2)");
  expect(light.get("--o-sel")).toBe("rgba(91, 75, 219, 0.22)");
  expect(light.get("--o-wordHover")).toBe("rgba(91, 75, 219, 0.12)");
});
