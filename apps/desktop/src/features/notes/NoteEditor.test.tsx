import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NoteEditor } from "./NoteEditor";

afterEach(cleanup);

it("P9-04 the editor shows the markdown as formatted text and writes markdown back", async () => {
  const onChange = vi.fn();
  render(
    <NoteEditor
      label="Notas"
      markdown={"## Plan\n\n- [ ] llamar\n- [x] **hecho**\n\nVer [acta](https://example.com)"}
      onChange={onChange}
      placeholder="Escribe…"
    />,
  );
  const box = await screen.findByRole("textbox", { name: "Notas" });
  await waitFor(() => expect(box.querySelector("h2")?.textContent).toBe("Plan"));
  expect(box.querySelector("strong")?.textContent).toBe("hecho");
  expect(box.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
  expect(box.querySelector("a")?.getAttribute("href")).toBe("https://example.com");
  expect(onChange).not.toHaveBeenCalled();
});

it("P9-04 hostile markdown stays text inside the editor", async () => {
  render(
    <NoteEditor
      label="Notas"
      markdown={'<script>alert(1)</script> [x](javascript:alert(1)) <img src=x onerror="alert(1)">'}
      onChange={() => undefined}
      placeholder="Escribe…"
    />,
  );
  const box = await screen.findByRole("textbox", { name: "Notas" });
  await waitFor(() => expect(box.textContent).toContain("<script>alert(1)</script>"));
  expect(box.querySelector("script, img")).toBeNull();
  expect(box.querySelector("a")).toBeNull();
});
