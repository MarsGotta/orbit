import { expect, it } from "vitest";
import type { Notice } from "../bridge/generated/Notice";
import { receive } from "./interceptor";

it.each([
  ["engine", "exit", 0, "El motor de control terminó sin que Orbit lo pidiera (código 0)."],
  ["engine", "exit", 23, "El motor de control terminó sin que Orbit lo pidiera (código 23)."],
  [
    "proxy",
    "signal",
    15,
    "El proceso de conexión del motor de control terminó sin que Orbit lo pidiera (señal 15).",
  ],
] as const)(
  "dice qué proceso terminó, cómo y con qué número: %s %s %s",
  (cause, code, status, text) => {
    const notice: Notice = {
      code: "engine_exited",
      op: "sim98-exit-notice",
      data: { cause, code, status },
    };
    const { messages } = receive([], { kind: "notice", notice });
    expect(messages).toEqual([
      {
        op: "sim98-exit-notice",
        class: "fixable",
        text: `${text} Orbit volverá a iniciarlo. No se repetirá ninguna acción de entrada.`,
      },
    ]);
  },
);
