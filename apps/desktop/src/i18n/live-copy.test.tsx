import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { CopyProvider, useCopy } from "./CopyProvider";
import { getCopy } from ".";
import { NoticeBanner } from "../app/NoticeBanner";
import { receive } from "../app/interceptor";
import { LanguageSetting } from "../features/settings/LanguageSetting";

function ReadCopy() {
  const copy = useCopy();
  return <p>{copy.shell.ask}</p>;
}

it("follows system language changes in mounted components and the plain getter", async () => {
  let preferred = ["es-ES"];
  vi.spyOn(navigator, "languages", "get").mockImplementation(() => preferred);
  render(
    <CopyProvider name="Atlas">
      <ReadCopy />
    </CopyProvider>,
  );
  expect(screen.getByText("Preguntar a Atlas")).toBeTruthy();
  preferred = ["fr-FR", "en-GB"];
  await act(async () => window.dispatchEvent(new Event("languagechange")));
  expect(screen.getByText("Ask Atlas")).toBeTruthy();
  expect(getCopy().shell.ask).toBe("Ask Atlas");
  expect(document.documentElement.lang).toBe("en");
});

it("keeps Spanish when the daemon refuses a language save", async () => {
  const send = vi.fn(async () => false);
  render(
    <CopyProvider choice="es">
      <LanguageSetting send={send} />
      <ReadCopy />
    </CopyProvider>,
  );
  await act(async () =>
    fireEvent.change(screen.getByRole("combobox", { name: "Idioma" }), {
      target: { value: "en" },
    }),
  );
  expect(screen.getByText("Preguntar a Deimos")).toBeTruthy();
  expect(screen.getByRole<HTMLSelectElement>("combobox", { name: "Idioma" }).value).toBe("es");
});

it("rewords an existing notice and its action without receiving it again", () => {
  const notice = receive([], {
    kind: "refusal",
    op: "language-fixture",
    refusal: { code: "not_paired", retry: "never" },
  });
  const props = {
    messages: notice.messages,
    onAction: vi.fn(),
    onDismiss: vi.fn(),
    onRetry: vi.fn(),
  };
  const view = render(
    <CopyProvider choice="es">
      <NoticeBanner {...props} />
    </CopyProvider>,
  );
  expect(view.container.textContent).toContain("emparej");
  view.rerender(
    <CopyProvider choice="en">
      <NoticeBanner {...props} />
    </CopyProvider>,
  );
  expect(view.container.textContent?.toLowerCase()).toContain("pair");
  expect(view.container.textContent).not.toContain("emparej");
  expect(props.onAction).not.toHaveBeenCalled();
});

it("accepts later daemon language changes after acknowledging a local choice", async () => {
  const send = vi.fn(async () => true);
  const content = (choice: "en" | "es") => (
    <CopyProvider choice={choice} name="Atlas">
      <LanguageSetting send={send} />
      <ReadCopy />
    </CopyProvider>
  );
  const view = render(content("en"));
  fireEvent.change(screen.getByRole("combobox", { name: "Language" }), { target: { value: "es" } });
  expect(await screen.findByText("Preguntar a Atlas")).toBeTruthy();
  view.rerender(content("es"));
  view.rerender(content("en"));
  expect(screen.getByText("Ask Atlas")).toBeTruthy();
});
