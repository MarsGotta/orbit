import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Attachment } from "../../bridge/generated/Attachment";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { MessageList } from "./MessageList";

// A message shows what it carried: images as thumbnails that open in a
// viewer, a PDF in WebKit's own viewer, any other file with «Abrir» and
// «Mostrar en Finder». A file that cannot be loaded says why.

const bridge = vi.hoisted(() => ({
  addAttachment: vi.fn(),
  attachmentBytes: vi.fn(),
  openAttachment: vi.fn(),
  chooseFiles: vi.fn(),
  onFilesDropped: vi.fn(),
  copyImage: vi.fn(),
}));
vi.mock("../../bridge/attachments", () => bridge);

const plano: Attachment = {
  id: "a-plano",
  name: "plano.png",
  kind: "image",
  mime: "image/png",
  bytes: 2_048,
};
const fachada: Attachment = {
  id: "a-fachada",
  name: "fachada.jpg",
  kind: "image",
  mime: "image/jpeg",
  bytes: 4_096,
};
const informe: Attachment = {
  id: "a-informe",
  name: "informe.pdf",
  kind: "pdf",
  mime: "application/pdf",
  bytes: 3 * 1024 * 1024,
  pages: 12,
};
const notas: Attachment = {
  id: "a-notas",
  name: "notas.txt",
  kind: "text",
  mime: "text/plain",
  bytes: 512,
};

function message(id: string, attachments: Attachment[]): ChatMessage {
  return {
    id,
    role: "user",
    text: "Adjunto",
    at_ms: 1_700_000_000_000,
    streaming: false,
    voice: null,
    receipts: [],
    attachments,
  };
}

const blobs: Blob[] = [];
const revoked: string[] = [];

beforeEach(() => {
  blobs.length = 0;
  revoked.length = 0;
  URL.createObjectURL = vi.fn((blob: Blob) => {
    blobs.push(blob);
    return `blob:orbit/${blobs.length}`;
  });
  URL.revokeObjectURL = vi.fn((url: string) => void revoked.push(url));
  bridge.attachmentBytes.mockImplementation((id: string) =>
    Promise.resolve(new TextEncoder().encode(id).buffer),
  );
  bridge.openAttachment.mockResolvedValue(undefined);
  bridge.copyImage.mockResolvedValue(undefined);
});

afterEach(() => {
  // Unmounting frees the blob URLs, so it comes before the stubs go.
  cleanup();
  Reflect.deleteProperty(URL, "createObjectURL");
  Reflect.deleteProperty(URL, "revokeObjectURL");
});

function show(messages: ChatMessage[]) {
  return render(<MessageList messages={messages} send={() => undefined} />);
}

describe("images", () => {
  it("shows thumbnails of the bytes, loaded once, and frees them when gone", async () => {
    const view = show([message("1", [plano, fachada])]);
    const thumbnail = await screen.findByRole("img", { name: "plano.png" });
    expect(thumbnail.getAttribute("src")).toMatch(/^blob:orbit\//);
    await screen.findByRole("img", { name: "fachada.jpg" });
    expect(bridge.attachmentBytes).toHaveBeenCalledTimes(2);
    expect(blobs.map((blob) => blob.type)).toEqual(["image/png", "image/jpeg"]);
    view.unmount();
    expect(revoked).toHaveLength(2);
  });

  it("opens the viewer, moves between the images of the conversation, copies and closes", async () => {
    show([message("1", [plano]), message("2", [notas, fachada])]);
    await userEvent.click(await screen.findByRole("button", { name: "Ver plano.png" }));
    const viewer = await screen.findByRole("dialog", { name: "plano.png" });
    expect(within(viewer).getByRole("button", { name: "Anterior" })).toHaveProperty(
      "disabled",
      true,
    );
    await userEvent.click(within(viewer).getByRole("button", { name: "Siguiente" }));
    await screen.findByRole("dialog", { name: "fachada.jpg" });
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "ArrowLeft" });
    await screen.findByRole("dialog", { name: "plano.png" });
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "ArrowRight" });
    const second = await screen.findByRole("dialog", { name: "fachada.jpg" });
    await userEvent.click(await within(second).findByRole("button", { name: "Copiar" }));
    expect(bridge.copyImage).toHaveBeenCalledWith(
      new TextEncoder().encode("a-fachada").buffer,
      "image/jpeg",
    );
    fireEvent.keyDown(second, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(bridge.attachmentBytes).toHaveBeenCalledTimes(2);
  });

  it("a thumbnail that cannot be loaded says why instead of a broken image", async () => {
    bridge.attachmentBytes.mockRejectedValue({
      refusal: {
        code: "attachment",
        cause: { kind: "not_found", attachment_id: "a-plano" },
        retry: { kind: "after_fix" },
      },
    });
    show([message("1", [plano])]);
    expect(await screen.findByText(/Ese adjunto ya no está en el hub/)).toBeTruthy();
    expect(screen.queryByRole("img", { name: "plano.png" })).toBeNull();
  });
});

describe("PDFs and other files", () => {
  it("shows a PDF's pages and size, and opens it in WebKit's own viewer", async () => {
    show([message("1", [informe])]);
    const pdf = screen.getByRole("group", { name: "informe.pdf" });
    expect(pdf.textContent).toContain("12 páginas");
    expect(pdf.textContent).toContain("3 MB");
    await userEvent.click(within(pdf).getByRole("button", { name: "Ver" }));
    const viewer = await screen.findByRole("dialog", { name: "informe.pdf" });
    const frame = await waitFor(() => {
      const element = viewer.querySelector("iframe");
      expect(element).not.toBeNull();
      return element as HTMLIFrameElement;
    });
    expect(frame.getAttribute("title")).toBe("informe.pdf");
    expect(frame.getAttribute("src")).toBe(`blob:orbit/${blobs.length}`);
    expect(blobs.at(-1)?.type).toBe("application/pdf");
  });

  it("opens a file with the Mac's app or shows it in Finder", async () => {
    show([message("1", [informe, notas])]);
    const text = screen.getByRole("group", { name: "notas.txt" });
    expect(text.textContent).toContain("512 B");
    await userEvent.click(within(text).getByRole("button", { name: "Abrir" }));
    await userEvent.click(within(text).getByRole("button", { name: "Mostrar en Finder" }));
    const pdf = screen.getByRole("group", { name: "informe.pdf" });
    await userEvent.click(within(pdf).getByRole("button", { name: "Mostrar en Finder" }));
    expect(bridge.openAttachment.mock.calls).toEqual([
      ["a-notas", false],
      ["a-notas", true],
      ["a-informe", true],
    ]);
  });
});
