import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { Attachment } from "../../bridge/generated/Attachment";
import type { AttachmentLimits } from "../../bridge/generated/AttachmentLimits";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { AssistantComposer } from "./AssistantComposer";
import AssistantView from "./AssistantView";

// The composer attaches files from the dialog, from a drop onto the window,
// from a pasted image, a long pasted text and the frontmost window. Each one
// is a chip before sending; the send carries the ready ones. A refusal names
// the file and its numbers; a hub without attachments offers none.

const bridge = vi.hoisted(() => ({
  addAttachment: vi.fn(),
  attachmentBytes: vi.fn(),
  openAttachment: vi.fn(),
  chooseFiles: vi.fn(),
  onFilesDropped: vi.fn(),
  copyImage: vi.fn(),
}));
vi.mock("../../bridge/attachments", () => bridge);

const MIB = 1024 * 1024;
const LIMITS: AttachmentLimits = {
  max_file_bytes: 20 * MIB,
  max_message_files: 10,
  max_message_bytes: 32 * MIB,
  max_session_bytes: 256 * MIB,
};

function stored(name: string, bytes = 2_048): Attachment {
  const kind = name.endsWith(".png") ? "image" : name.endsWith(".pdf") ? "pdf" : "text";
  const mime = { image: "image/png", pdf: "application/pdf", text: "text/plain" }[kind];
  return { id: `adjunto-${name}`, name, kind, mime, bytes };
}

type Source = { from: "path"; path: string } | { from: "bytes"; name: string } | { from: "window" };

let dropped: (paths: string[]) => void = () => undefined;

beforeEach(() => {
  dropped = () => undefined;
  bridge.addAttachment.mockImplementation((source: Source) => {
    if (source.from === "path")
      return Promise.resolve({ attachment: stored(source.path.split("/").pop()!) });
    if (source.from === "bytes") return Promise.resolve({ attachment: stored(source.name) });
    return Promise.resolve({ attachment: stored("Ventana de Notas.png") });
  });
  bridge.chooseFiles.mockResolvedValue([]);
  bridge.onFilesDropped.mockImplementation((handler: (paths: string[]) => void) => {
    dropped = handler;
    return Promise.resolve(() => undefined);
  });
});

const chat: ChatSnapshot = {
  turn: [],
  pending: null,
  session_id: null,
  voice: { level: 0, state: "idle" },
};

function renderComposer(limits: AttachmentLimits | null = LIMITS) {
  const send = vi.fn<(command: AppCommand) => unknown>();
  render(
    <AssistantComposer
      attachmentLimits={limits ?? undefined}
      callActive={false}
      callPending={false}
      chat={chat}
      daemonReady
      hub="online"
      onCallToggle={() => undefined}
      paired
      send={send}
    />,
  );
  return { send, box: screen.getByRole("textbox", { name: "Mensaje" }) };
}

const sendButton = () => screen.getByRole("button", { name: "Enviar" });
const chip = (name: string) => screen.getByRole("group", { name });

describe("attaching from the dialog", () => {
  it("shows a chip per chosen file and sends them with the text", async () => {
    bridge.chooseFiles.mockResolvedValue([
      "/Users/usuaria/presupuesto.pdf",
      "/Users/usuaria/plano.png",
    ]);
    const sent: AppCommand[] = [];
    render(
      <AssistantView
        error={null}
        send={(command) => sent.push(command)}
        snapshot={
          {
            connection: { daemon: "ready", hub: "paired" },
            status_availability: { state: "current" },
            call_availability: { state: "current" },
            call: null,
            settings: { hub: { paired: true, connection: "online" }, attachments: LIMITS },
            chat: { ...chat, availability: { state: "current" } },
          } as unknown as AppSnapshot
        }
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar" }));
    await waitFor(() => expect(chip("presupuesto.pdf").textContent).toContain("2 KB"));
    expect(bridge.addAttachment).toHaveBeenCalledWith({
      from: "path",
      path: "/Users/usuaria/presupuesto.pdf",
    });
    await userEvent.type(screen.getByRole("textbox", { name: "Mensaje" }), "Míralos");
    await userEvent.click(sendButton());
    expect(sent).toEqual([
      {
        command: "chat",
        text: "Míralos",
        attachments: [stored("presupuesto.pdf"), stored("plano.png")],
      },
    ]);
    expect(screen.queryByRole("group", { name: "plano.png" })).toBeNull();
  });

  it("sends attachments without text, but not while one is still uploading", async () => {
    let finish: (added: { attachment: Attachment }) => void = () => undefined;
    bridge.addAttachment.mockImplementation(
      () => new Promise<{ attachment: Attachment }>((resolve) => (finish = resolve)),
    );
    bridge.chooseFiles.mockResolvedValue(["/Users/usuaria/notas.txt"]);
    const { send } = renderComposer();
    expect(sendButton()).toHaveProperty("disabled", true);
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar" }));
    expect((await screen.findByRole("group", { name: "notas.txt" })).textContent).toContain(
      "Subiendo…",
    );
    expect(sendButton()).toHaveProperty("disabled", true);
    await act(async () => finish({ attachment: stored("notas.txt") }));
    expect(sendButton()).toHaveProperty("disabled", false);
    await userEvent.click(sendButton());
    expect(send).toHaveBeenCalledWith({
      command: "chat",
      text: "",
      attachments: [stored("notas.txt")],
    });
  });

  it("attaches the frontmost window", async () => {
    const { send } = renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar ventana" }));
    expect(bridge.addAttachment).toHaveBeenCalledWith({ from: "window" });
    await screen.findByRole("group", { name: "Ventana de Notas.png" });
    await userEvent.click(sendButton());
    expect(send.mock.calls[0]?.[0]).toMatchObject({
      attachments: [stored("Ventana de Notas.png")],
    });
  });

  it("attaches the window's text next to its capture", async () => {
    bridge.addAttachment.mockResolvedValue({
      attachment: stored("Ventana de Notas.png"),
      window_text: { state: "attached", attachment: stored("Ventana de Notas.txt", 300) },
    });
    const { send } = renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar ventana" }));
    await screen.findByRole("group", { name: "Ventana de Notas.txt" });
    expect(chip("Ventana de Notas.png").textContent).toContain("2 KB");
    await userEvent.click(sendButton());
    expect(send.mock.calls[0]?.[0]).toMatchObject({
      attachments: [stored("Ventana de Notas.png"), stored("Ventana de Notas.txt", 300)],
    });
  });

  it("says when a window without readable text goes as its capture alone", async () => {
    bridge.addAttachment.mockResolvedValue({
      attachment: stored("Ventana de Notas.png"),
      window_text: { state: "missing", cause: "empty" },
    });
    renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar ventana" }));
    await waitFor(() =>
      expect(chip("Ventana de Notas.png").textContent).toContain(
        "La ventana no tiene texto legible; va sólo la captura.",
      ),
    );
    expect(screen.queryByRole("group", { name: "Ventana de Notas.txt" })).toBeNull();
  });

  it("names the hub's refusal of the window's text", async () => {
    bridge.addAttachment.mockResolvedValue({
      attachment: stored("Ventana de Notas.png"),
      window_text: {
        state: "refused",
        refusal: {
          code: "attachment",
          cause: { kind: "too_many", files: 11, max_files: 10 },
          retry: { kind: "after_fix" },
        },
      },
    });
    renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar ventana" }));
    await waitFor(() =>
      expect(chip("Ventana de Notas.png").textContent).toContain(
        "Su texto no se adjuntó: Este mensaje llevaría 11 adjuntos y el máximo por mensaje es 10.",
      ),
    );
  });

  it("removes a chip, which is then not sent", async () => {
    bridge.chooseFiles.mockResolvedValue(["/Users/usuaria/notas.txt", "/Users/usuaria/plano.png"]);
    const { box, send } = renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar" }));
    await screen.findByRole("group", { name: "plano.png" });
    await userEvent.click(screen.getByRole("button", { name: "Quitar notas.txt" }));
    expect(screen.queryByRole("group", { name: "notas.txt" })).toBeNull();
    await userEvent.type(box, "Sólo el plano{Enter}");
    expect(send).toHaveBeenCalledWith({
      command: "chat",
      text: "Sólo el plano",
      attachments: [stored("plano.png")],
    });
  });
});

describe("attaching by dropping files on the window", () => {
  it("uploads the dropped paths", async () => {
    renderComposer();
    await waitFor(() => expect(bridge.onFilesDropped).toHaveBeenCalled());
    act(() => dropped(["/Users/usuaria/Escritorio/plano.png"]));
    await screen.findByRole("group", { name: "plano.png" });
    expect(bridge.addAttachment).toHaveBeenCalledWith({
      from: "path",
      path: "/Users/usuaria/Escritorio/plano.png",
    });
  });
});

function paste(box: HTMLElement, data: { files?: File[]; text?: string }) {
  return fireEvent.paste(box, {
    clipboardData: {
      files: data.files ?? [],
      types: data.files ? ["Files"] : ["text/plain"],
      getData: (type: string) => (type === "text/plain" ? (data.text ?? "") : ""),
    },
  });
}

describe("pasting", () => {
  it("an image becomes an attachment of its bytes", async () => {
    const { box } = renderComposer();
    const image = new File([new Uint8Array([137, 80, 78, 71])], "", { type: "image/png" });
    expect(paste(box, { files: [image] })).toBe(false);
    await screen.findByRole("group", { name: "Imagen pegada.png" });
    expect(bridge.addAttachment).toHaveBeenCalledWith({
      from: "bytes",
      name: "Imagen pegada.png",
      data_base64: "iVBORw==",
    });
  });

  it("a long text becomes «Texto pegado.txt» instead of filling the box", async () => {
    const { box } = renderComposer();
    const long = "Una línea del acta.\n".repeat(250);
    expect(paste(box, { text: long })).toBe(false);
    await screen.findByRole("group", { name: "Texto pegado.txt" });
    const source = bridge.addAttachment.mock.calls[0]?.[0] as { data_base64: string };
    const bytes = Uint8Array.from(atob(source.data_base64), (char) => char.charCodeAt(0));
    expect(new TextDecoder().decode(bytes)).toBe(long);
    expect((box as HTMLTextAreaElement).value).toBe("");
  });

  it("a short text goes into the box as usual", () => {
    const { box } = renderComposer();
    expect(paste(box, { text: "Hola" })).toBe(true);
    expect(bridge.addAttachment).not.toHaveBeenCalled();
  });
});

describe("refusals", () => {
  it("a refused upload names the file and its type, and can be removed", async () => {
    bridge.addAttachment.mockRejectedValue({
      refusal: {
        code: "attachment",
        cause: { kind: "type_unsupported", name: "foto.heic", mime: "image/heic" },
        retry: { kind: "after_fix" },
      },
    });
    bridge.chooseFiles.mockResolvedValue(["/Users/usuaria/foto.heic"]);
    const { send } = renderComposer();
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar" }));
    await waitFor(() =>
      expect(chip("foto.heic").textContent).toContain("“foto.heic” es image/heic"),
    );
    expect(sendButton()).toHaveProperty("disabled", true);
    await userEvent.click(
      within(chip("foto.heic")).getByRole("button", { name: "Quitar foto.heic" }),
    );
    expect(screen.queryByRole("group", { name: "foto.heic" })).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("a 0.9.0 hub offers no attaching and says why", async () => {
    const { box } = renderComposer(null);
    for (const name of ["Adjuntar", "Adjuntar ventana"]) {
      const button = screen.getByRole("button", { name });
      expect(button).toHaveProperty("disabled", true);
      expect(button.getAttribute("title")).toBe("Este hub no admite adjuntos");
    }
    const image = new File([new Uint8Array([1])], "captura.png", { type: "image/png" });
    paste(box, { files: [image] });
    await waitFor(() =>
      expect(chip("captura.png").textContent).toContain(
        "Este hub no admite adjuntos; actualízalo.",
      ),
    );
    act(() => dropped(["/Users/usuaria/plano.png"]));
    expect(chip("plano.png").textContent).toContain("Este hub no admite adjuntos");
    expect(bridge.addAttachment).not.toHaveBeenCalled();
  });

  it("checks the hub's per-message ceilings before uploading", async () => {
    bridge.chooseFiles.mockResolvedValue([
      "/Users/usuaria/uno.txt",
      "/Users/usuaria/dos.txt",
      "/Users/usuaria/tres.txt",
    ]);
    const { box } = renderComposer({ ...LIMITS, max_message_files: 2, max_file_bytes: 3 });
    await userEvent.click(screen.getByRole("button", { name: "Adjuntar" }));
    await waitFor(() =>
      expect(chip("tres.txt").textContent).toContain(
        "Este mensaje llevaría 3 adjuntos y el máximo por mensaje es 2.",
      ),
    );
    expect(bridge.addAttachment).toHaveBeenCalledTimes(2);
    await userEvent.click(screen.getByRole("button", { name: "Quitar dos.txt" }));
    const image = new File([new Uint8Array([1, 2, 3, 4])], "grande.png", { type: "image/png" });
    paste(box, { files: [image] });
    await waitFor(() =>
      expect(chip("grande.png").textContent).toContain(
        "“grande.png” ocupa 4 B y el máximo por fichero es 3 B.",
      ),
    );
    expect(bridge.addAttachment).toHaveBeenCalledTimes(2);
  });
});
