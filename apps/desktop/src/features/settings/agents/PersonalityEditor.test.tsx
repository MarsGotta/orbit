import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { editorNamed, focusAt, lines, typeText } from "../../editor/test-editor";
import PersonalityEditor from "./PersonalityEditor";

afterEach(cleanup);

// A personality is Markdown with whatever it carries; the editor is the one
// every note opens in, with code blocks, quotes and tables. Invented text.
const PERSONALITY = [
  "# Vega",
  "",
  "> Sé breve.",
  "",
  "```sh",
  "orbit --version",
  "```",
  "",
  "| Día | Tono |",
  "| --- | --- |",
  "| Lunes | Sobrio |",
].join("\n");

it("shows the personality formatted, code and table included, and sends nothing unasked", () => {
  const onChange = vi.fn();
  render(
    <PersonalityEditor
      editable
      label="Personalidad de Vega"
      markdown={PERSONALITY}
      onChange={onChange}
    />,
  );
  const editor = editorNamed("Personalidad de Vega");
  const shown = lines(editor);
  expect(shown[0]).toMatchObject({ text: "Vega" });
  expect(shown[0]?.classes).toContain("cm-md-h1");
  expect(shown[2]).toMatchObject({ text: "Sé breve." });
  expect(shown[5]).toMatchObject({ text: "orbit --version" });
  expect(shown[5]?.classes).toContain("cm-md-codeblock");
  expect(shown[8]?.text).toBe("| Día | Tono |");
  expect(editor.contentDOM.getAttribute("contenteditable")).toBe("true");
  expect(onChange).not.toHaveBeenCalled();
});

it("sends the Markdown as typed once the user changes it", () => {
  const onChange = vi.fn();
  render(
    <PersonalityEditor
      editable
      label="Personalidad de Vega"
      markdown="Sé breve."
      onChange={onChange}
    />,
  );
  const editor = editorNamed("Personalidad de Vega");
  focusAt(editor);
  typeText(editor, " **Mucho**.");
  expect(onChange).toHaveBeenLastCalledWith("Sé breve. **Mucho**.");
});

it("a personality the agent's kind does not let Orbit change is shown read-only", () => {
  render(
    <PersonalityEditor
      editable={false}
      label="Personalidad de Atlas"
      markdown="Soy **Atlas**."
      onChange={() => undefined}
    />,
  );
  const editor = editorNamed("Personalidad de Atlas");
  expect(editor.contentDOM.querySelector(".cm-md-strong")?.textContent).toBe("Atlas");
  expect(editor.contentDOM.getAttribute("contenteditable")).toBe("false");
});
