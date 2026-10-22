import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import PersonalityEditor from "./PersonalityEditor";

afterEach(cleanup);

// A personality is Markdown with whatever it carries; the editor is the one
// answers open in, with code blocks, quotes and tables. Invented text.
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

it("shows the personality formatted, code and table included, and sends nothing unasked", async () => {
  const onChange = vi.fn();
  render(
    <PersonalityEditor
      editable
      label="Personalidad de Vega"
      markdown={PERSONALITY}
      onChange={onChange}
    />,
  );
  const box = await screen.findByRole("textbox", { name: "Personalidad de Vega" });
  await waitFor(() => expect(box.querySelector("h1")?.textContent).toBe("Vega"));
  expect(box.querySelector("blockquote")?.textContent).toBe("Sé breve.");
  expect(box.querySelector("pre code")?.textContent).toBe("orbit --version");
  expect(box.querySelector("table")).not.toBeNull();
  expect(box.getAttribute("contenteditable")).toBe("true");
  expect(onChange).not.toHaveBeenCalled();
});

it("a personality the agent's kind does not let Orbit change is shown read-only", async () => {
  render(
    <PersonalityEditor
      editable={false}
      label="Personalidad de Atlas"
      markdown="Soy **Atlas**."
      onChange={() => undefined}
    />,
  );
  const box = await screen.findByRole("textbox", { name: "Personalidad de Atlas" });
  await waitFor(() => expect(box.querySelector("strong")?.textContent).toBe("Atlas"));
  expect(box.getAttribute("contenteditable")).toBe("false");
});
