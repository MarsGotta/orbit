import { expect, it } from "vitest";
import fontCss from "./fonts.css?inline";
import tokenCss from "./tokens.css?inline";

it("no pide fuentes a Google desde el webview", () => {
  expect(`${fontCss}\n${tokenCss}`).not.toMatch(/fonts\.(?:googleapis|gstatic)\.com/);
});
