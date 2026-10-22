import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ChatMarkdown } from "./ChatMarkdown";

const copied = vi.hoisted(() => ({ texts: [] as string[] }));
vi.mock("../../bridge/clipboard", () => ({
  copyText: (text: string) => {
    copied.texts.push(text);
    return Promise.resolve();
  },
}));

// The answers of the assistant are Markdown: code with its language and
// «Copiar», tables that scroll and copy as Markdown, quotes, rules and inline
// code. Nothing the model writes becomes HTML.

const CODE = "```ts\nconst total = 3;\nconsole.log(total);\n```";
const TABLE = "| Sala | Plazas |\n| --- | ---: |\n| Norte | 8 |\n| Sur | 12 |";

describe("code blocks", () => {
  it("shows the language and copies exactly the code", async () => {
    copied.texts = [];
    render(<ChatMarkdown source={`Mira:\n\n${CODE}\n\nListo.`} />);
    const block = screen.getByRole("figure");
    expect(within(block).getByText("ts")).toBeTruthy();
    expect(block.querySelector("pre code")?.textContent).toBe(
      "const total = 3;\nconsole.log(total);",
    );
    await userEvent.click(within(block).getByRole("button", { name: "Copiar código" }));
    expect(copied.texts).toEqual(["const total = 3;\nconsole.log(total);"]);
    expect(await within(block).findByText("✓ Copiado")).toBeTruthy();
  });

  it("keeps a block's state when the answer is drawn again unchanged", async () => {
    const { rerender } = render(<ChatMarkdown source={CODE} />);
    await userEvent.click(screen.getByRole("button", { name: "Copiar código" }));
    expect(await screen.findByText("✓ Copiado")).toBeTruthy();
    rerender(<ChatMarkdown source={CODE} />);
    expect(screen.getByText("✓ Copiado")).toBeTruthy();
  });

  it("colours a block with a known language once the highlighter loads", async () => {
    render(<ChatMarkdown source={CODE} />);
    const code = screen.getByRole("figure").querySelector("pre code");
    await waitFor(() => expect(code?.querySelector("span[style*='--shiki-dark']")).not.toBeNull());
    expect(code?.textContent).toBe("const total = 3;\nconsole.log(total);");
  });

  it("leaves a block without a language as plain text, with its copy button", () => {
    render(<ChatMarkdown source={"```\nnpm test\n```"} />);
    const block = screen.getByRole("figure");
    expect(block.querySelector("pre code span")).toBeNull();
    expect(within(block).getByRole("button", { name: "Copiar código" })).toBeTruthy();
  });

  it("draws inline code, quotes and rules", () => {
    const { container } = render(
      <ChatMarkdown source={"Usa `pnpm test`.\n\n> Citado\n\n---\n\nFin"} />,
    );
    expect(container.querySelector("p code")?.textContent).toBe("pnpm test");
    expect(container.querySelector("blockquote")?.textContent).toContain("Citado");
    expect(container.querySelector("hr")).not.toBeNull();
    expect(screen.queryByRole("figure")).toBeNull();
  });
});

describe("tables", () => {
  it("scrolls sideways and copies the table as Markdown", async () => {
    copied.texts = [];
    render(<ChatMarkdown source={`Las salas:\n\n${TABLE}\n\nEso es todo.`} />);
    const table = screen.getByRole("table");
    expect(within(table).getByText("Norte")).toBeTruthy();
    expect(table.parentElement?.getAttribute("data-scroll")).toBe("x");
    await userEvent.click(screen.getByRole("button", { name: "Copiar tabla" }));
    expect(copied.texts).toEqual([TABLE]);
  });
});

describe("states in a table", () => {
  it("colours a cell that says only a state, and nothing else", () => {
    render(
      <ChatMarkdown
        source={
          "| Hotel | Formación | Estado |\n| --- | --- | --- |\n| Norte | Hecha | Listo |\n| Sur | Martes | Pendiente |\n| Este | Sin fecha | Bloqueado |"
        }
      />,
    );
    const state = (text: string) => screen.getByText(text).closest("td")?.dataset.state;
    expect([state("Listo"), state("Pendiente"), state("Bloqueado")]).toEqual(["ok", "warn", "rec"]);
    expect(state("Hecha")).toBeUndefined();
    expect(state("Norte")).toBeUndefined();
  });
});

describe("nothing the model writes becomes HTML", () => {
  it("shows raw HTML as the text it is", () => {
    const { container } = render(
      <ChatMarkdown
        source={
          'Hola <b>fuerte</b>\n\n<img src="x" onerror="alert(1)">\n\n<script>alert(1)</script>'
        }
      />,
    );
    expect(container.querySelector("b, img, script")).toBeNull();
    expect(container.textContent).toContain("<b>fuerte</b>");
    expect(container.textContent).toContain('<img src="x" onerror="alert(1)">');
  });

  it("links only the web, and never loads an image", () => {
    const { container } = render(
      <ChatMarkdown
        source={
          "[web](https://example.test/a) [mal](javascript:alert(1)) ![logo](https://example.test/x.png)"
        }
      />,
    );
    const links = screen.getAllByRole("link");
    expect(links.map((link) => link.getAttribute("href"))).toEqual(["https://example.test/a"]);
    expect(screen.getByText("mal").closest("a")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("logo");
  });

  it("draws a link to an Orbit view as a chip that opens it", () => {
    render(<ChatMarkdown source={"Está en [la nota de la revisión](#/notes/rec-ficticia)."} />);
    const chip = screen.getByRole("link", { name: "la nota de la revisión" });
    expect(chip.getAttribute("href")).toBe("#/notes/rec-ficticia");
    expect(chip.getAttribute("data-chip")).toBe("note");
    expect(chip.getAttribute("target")).toBeNull();
  });
});
