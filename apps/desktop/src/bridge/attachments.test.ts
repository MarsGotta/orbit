import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import {
  addAttachment,
  attachmentBytes,
  chooseFiles,
  copyImage,
  onFilesDropped,
  openAttachment,
} from "./attachments";

afterEach(clearMocks);

/** The daemon's three attachment orders, with the arguments it reads. */
it("calls the daemon's attachment commands with their arguments", async () => {
  const calls: { command: string; payload: unknown }[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return command === "orbit_attachment_content" ? new ArrayBuffer(2) : undefined;
  });
  await addAttachment({ from: "path", path: "/Users/usuaria/plano.png" });
  await addAttachment({ from: "bytes", name: "Texto pegado.txt", data_base64: "SG9sYQ==" });
  await addAttachment({ from: "window" });
  expect((await attachmentBytes("adjunto-1")).byteLength).toBe(2);
  await openAttachment("adjunto-1", true);
  expect(calls).toEqual([
    {
      command: "orbit_attachment_add",
      payload: { source: { from: "path", path: "/Users/usuaria/plano.png" } },
    },
    {
      command: "orbit_attachment_add",
      payload: { source: { from: "bytes", name: "Texto pegado.txt", data_base64: "SG9sYQ==" } },
    },
    { command: "orbit_attachment_add", payload: { source: { from: "window" } } },
    { command: "orbit_attachment_content", payload: { attachmentId: "adjunto-1" } },
    { command: "orbit_attachment_open", payload: { attachmentId: "adjunto-1", reveal: true } },
  ]);
});

/** A dialog closed without choosing chooses nothing. */
it("asks the dialog for several files, and a cancelled one is none", async () => {
  const options: unknown[] = [];
  let answer: string[] | null = ["/Users/usuaria/notas.txt"];
  mockIPC((command, payload) => {
    if (command === "plugin:dialog|open") {
      options.push((payload as { options: unknown }).options);
      return answer;
    }
  });
  expect(await chooseFiles()).toEqual(["/Users/usuaria/notas.txt"]);
  answer = null;
  expect(await chooseFiles()).toEqual([]);
  expect(options[0]).toMatchObject({ multiple: true, directory: false });
});

/** Only a drop with files reaches the composer; hovering does not. */
it("reports only the paths of a drop", async () => {
  mockWindows("main");
  mockIPC(() => undefined, { shouldMockEvents: true });
  const handler = vi.fn();
  const unlisten = await onFilesDropped(handler);
  await emit("tauri://drag-enter", {
    paths: ["/Users/usuaria/plano.png"],
    position: { x: 1, y: 1 },
  });
  await emit("tauri://drag-drop", {
    paths: ["/Users/usuaria/plano.png"],
    position: { x: 1, y: 1 },
  });
  expect(handler.mock.calls).toEqual([[["/Users/usuaria/plano.png"]]]);
  unlisten();
});

/**
 * Copying decodes the image in the webview and hands its pixels to the
 * clipboard plugin as an image resource, which is freed afterwards.
 */
it("copies an image as its RGBA pixels, by resource", async () => {
  const pixels = new Uint8ClampedArray([255, 0, 0, 255, 0, 255, 0, 255]);
  vi.stubGlobal(
    "createImageBitmap",
    vi.fn(() => Promise.resolve({ width: 2, height: 1, close: () => undefined })),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    drawImage: () => undefined,
    getImageData: () => ({ data: pixels }),
  } as unknown as CanvasRenderingContext2D);
  const calls: { command: string; payload: unknown }[] = [];
  mockIPC((command, payload) => {
    calls.push({ command, payload });
    return command === "plugin:image|new" ? 41 : undefined;
  });
  await copyImage(new ArrayBuffer(4), "image/png");
  vi.unstubAllGlobals();
  expect(calls.map((call) => call.command)).toEqual([
    "plugin:image|new",
    "plugin:clipboard-manager|write_image",
    "plugin:resources|close",
  ]);
  expect(calls[0]?.payload).toMatchObject({ width: 2, height: 1 });
  expect(calls[1]?.payload).toEqual({ image: 41 });
});
