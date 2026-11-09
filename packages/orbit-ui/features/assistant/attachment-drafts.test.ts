import { describe, expect, it } from "vitest";
import type { AddedAttachment } from "../../bridge/generated/AddedAttachment";
import type { AttachmentLimits } from "../../bridge/generated/AttachmentLimits";
import { settled, type DraftAttachment } from "./attachment-drafts";

const MIB = 1024 * 1024;
const LIMITS: AttachmentLimits = {
  max_file_bytes: 20 * MIB,
  max_message_files: 10,
  max_message_bytes: 32 * MIB,
  max_session_bytes: 256 * MIB,
};

describe("a removed window chip", () => {
  it("does not bring its text back when the upload answers", () => {
    // Chip 0 was «Ventana», uploading; the person removed it (×) before the
    // answer, so the drafts no longer hold key 0.
    const drafts: DraftAttachment[] = [];
    const added: AddedAttachment = {
      attachment: {
        id: "adjunto-captura",
        name: "Editor — Nota sintética.png",
        kind: "image",
        mime: "image/png",
        bytes: 2_048,
      },
      window_text: {
        state: "attached",
        attachment: {
          id: "adjunto-texto",
          name: "Editor — Nota sintética.txt",
          kind: "text",
          mime: "text/plain",
          bytes: 120,
        },
      },
    };
    let key = 1;
    const after = settled(LIMITS, drafts, 0, added, () => key++);
    // Nothing the person removed may be sent with the next message.
    expect(after).toEqual([]);
  });
});
