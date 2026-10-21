import { expect, it } from "vitest";
import fontCss from "./fonts.css?inline";
import tokenCss from "./tokens.css?inline";

it("does not ask Google for fonts from the webview", () => {
  expect(`${fontCss}\n${tokenCss}`).not.toMatch(/fonts\.(?:googleapis|gstatic)\.com/);
});
