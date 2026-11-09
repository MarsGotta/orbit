import { expect, it, vi } from "vitest";
import { chooseFiles } from "orbit-ui/bridge/attachments";
import { copyText } from "orbit-ui/bridge/clipboard";

const tauri = vi.hoisted(() => ({
  writeText: vi.fn(() => Promise.resolve()),
  open: vi.fn(() => Promise.resolve(["/tmp/informe-de-prueba.pdf"])),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: tauri.writeText }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: tauri.open }));

// The entry runs in no other test and `tsc` does not run it: only here would
// a frame that stopped installing these be noticed before the real app. There
// is no #root in this document, so importing the entry mounts nothing.
it("plugs the Mac's clipboard and file dialog into the shared package", async () => {
  await import("./main");

  await copyText("texto de prueba");
  expect(tauri.writeText).toHaveBeenCalledWith("texto de prueba");

  await expect(chooseFiles()).resolves.toEqual(["/tmp/informe-de-prueba.pdf"]);
});
