import { ATTACHMENT_NOT_FOUND, sessionCeilingText, tooManyText } from "../attachments";
import type { Entries, NoticeData } from "./types";

/** A notice's number, which the hub sends as a decimal string. */
function amount(value: NoticeData[keyof NoticeData]): number | undefined {
  const parsed = typeof value === "string" ? Number(value) : value;
  return typeof parsed === "number" && Number.isFinite(parsed) ? parsed : undefined;
}

/**
 * A turn the hub refused for its files, in the words of the rule that fired;
 * any other cause of a failed turn has none and stays the generic failure.
 */
function attachmentFailure(data: NoticeData): string | undefined {
  switch (data.code) {
    case "attachments_too_large":
      return sessionCeilingText(amount(data.stored), amount(data.bytes), amount(data.max_bytes));
    case "attachments_too_many":
      return tooManyText(amount(data.count) ?? "?", amount(data.max) ?? "?");
    case "attachment_not_found":
      return ATTACHMENT_NOT_FOUND;
    case "attachment_refused":
      return "El agente no aceptó una de las imágenes de este mensaje; quítala o prueba con otra.";
    case "attachment_unreadable":
      return "El hub no pudo preparar uno de los adjuntos de este mensaje para el agente; quítalo o vuelve a adjuntarlo.";
    default:
      return undefined;
  }
}

export const assistantFailed: Entries<"assistant_failed"> = {
  assistant_failed: { class: "failed", text: attachmentFailure },
};
