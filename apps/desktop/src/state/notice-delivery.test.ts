import { expect, it, vi } from "vitest";
import type { AppBootstrap, FeatureFrame } from "../bridge/generated";
import type { Notice } from "../bridge/generated/Notice";
import type { DaemonPort } from "../bridge/port";
import callFrames from "../../../../contracts/fixtures/call-notice-frames.json";
import { createOrbitStore } from "./orbit-store";

function status(notices: Notice[], counter: number, epoch = "daemon-one"): FeatureFrame {
  return {
    feature: "status",
    revision: { epoch: 1, counter },
    view: { notices, notice: notices.at(-1) ?? null, notice_epoch: epoch, settings: {} },
  } as FeatureFrame;
}

async function connected() {
  let accept: (frame: FeatureFrame) => void = () => {
    throw new Error("Suscripción ausente");
  };
  const port: DaemonPort = {
    subscribeFrames: async (listener) => {
      accept = listener;
      return () => undefined;
    },
    snapshot: async () =>
      ({
        revision: { epoch: 1, counter: 1 },
        frames: [
          status([], 1),
          ...[
            "permissions",
            "agenda",
            "notes",
            "note",
            "recording",
            "assistant",
            "control",
            "briefing",
            "local",
          ].map((feature) => ({
            feature,
            revision: { epoch: 1, counter: 1 },
            view: feature === "notes" ? null : {},
          })),
        ],
      }) as AppBootstrap,
    send: vi.fn(),
    report: vi.fn(async () => undefined),
  };
  const store = createOrbitStore(port);
  await store.getState().connect();
  expect(store.getState().error).toBeNull();
  return { store, accept: (frame: FeatureFrame) => accept(frame) };
}

it("el cursor no resucita una transcripción cerrada al completarse el op de otro turno", async () => {
  const { store, accept } = await connected();
  const working = { code: "voice_transcribing", op: "call-first-turn", sequence: 3 } as Notice;
  // The same late op assignment measured at the daemon's core boundary.
  const thinking = { code: "assistant_thinking", sequence: 4 } as Notice;
  accept(status([working], 2));
  expect(store.getState().messages.map((message) => message.class)).toEqual(["working"]);
  accept(status([working, thinking], 3));
  store.getState().dismiss("call-first-turn");
  accept(status([working, { ...thinking, op: "call-following-turn" }], 4));
  expect(store.getState().messages.filter((message) => message.class === "working")).toEqual([]);
});

it("un aviso idéntico de la misma operación vuelve a verse cuando tiene otra secuencia", async () => {
  const { store, accept } = await connected();
  const notice = { code: "voice_transcribing", op: "call-turn", sequence: 1 } as Notice;
  accept(status([notice], 2));
  store.getState().dismiss("call-turn");
  accept(status([{ ...notice, sequence: 2 } as Notice], 3));
  expect(store.getState().messages.map((message) => message.op)).toEqual(["call-turn"]);
});

it("un daemon nuevo puede empezar con secuencias menores sin perder sus avisos", async () => {
  const { store, accept } = await connected();
  accept(status([{ code: "assistant_thinking", op: "old-call-turn", sequence: 99 } as Notice], 2));
  accept(
    status(
      [{ code: "voice_transcribing", op: "new-call-turn", sequence: 1 } as Notice],
      3,
      "daemon-two",
    ),
  );
  expect(store.getState().messages.map((message) => message.op)).toEqual(["new-call-turn"]);
});

it("los tres turnos de llamada del daemon terminan sin transcripciones colgadas", async () => {
  const { store, accept } = await connected();
  for (const [index, frame] of callFrames.entries()) {
    accept(status(frame.notices as Notice[], index + 2));
    expect(store.getState().messages.filter((message) => message.class === "working")).toHaveLength(
      index % 2 === 0 ? 1 : 0,
    );
  }
});

it("un aviso sin op sigue siendo desconocido hasta recibir su identidad", async () => {
  const { store, accept } = await connected();
  const notice = { code: "voice_transcribing", sequence: 1 } as Notice;
  accept(status([notice], 2));
  expect(store.getState().messages).toEqual([]);
  accept(status([{ ...notice, op: "identified-call-turn" }], 3));
  expect(store.getState().messages.map((message) => message.op)).toEqual(["identified-call-turn"]);
});

it("un registro atrasado no reduce el cursor ni repite trabajos ya cerrados", async () => {
  const { store, accept } = await connected();
  const current = { code: "voice_transcribing", op: "current-call-turn", sequence: 20 } as Notice;
  accept(status([current], 2));
  store.getState().dismiss("current-call-turn");
  accept(status([{ ...current, op: "old-call-turn", sequence: 10 }], 3));
  expect(store.getState().messages).toEqual([]);
  accept(status([current], 4));
  expect(store.getState().messages).toEqual([]);
});

it("el turno que desapareció se explica con su minuto y los turnos de ahora", async () => {
  const { store, accept } = await connected();
  const data = {
    code: "turn_no_longer_exists",
    step: "rename",
    status: 409,
    at_ms: 12000,
    turns: 3,
  };
  const notice: Notice = {
    code: "turn_no_longer_exists",
    op: "rename-lost-turn",
    sequence: 1,
    data,
  };
  accept(status([notice], 2));
  expect(store.getState().messages).toEqual([
    {
      op: "rename-lost-turn",
      class: "fixable",
      text: "Ese turno (0:12) ya no está: la transcripción cambió desde que la abriste y ahora tiene 3 turnos. Ábrela de nuevo.",
    },
  ]);
});

it.each([
  [
    { code: "recording_still_open", status: 400 },
    "No se puede volver a transcribir: la grabación todavía no se ha cerrado.",
  ],
  [
    { code: "diarization_not_configured", status: 409 },
    "No se puede volver a transcribir: el hub no tiene configurada la separación de voces.",
  ],
  [
    { code: "unexpected_refusal", status: 409 },
    "El hub rechazó la petición de volver a transcribir. (unexpected_refusal)",
  ],
])("una negativa de transcripción dice qué pasa: %j", async (data, text) => {
  const { store, accept } = await connected();
  accept(
    status(
      [{ code: "transcript_retranscribe_refused", op: "retranscribe-refused", sequence: 1, data }],
      2,
    ),
  );
  expect(store.getState().messages).toEqual([
    { op: "retranscribe-refused", class: "fixable", text },
  ]);
});

it.each([
  [{ cause: "hub_unreachable" }, "no se pudo conectar con el hub"],
  [
    { cause: "hub_response_too_large", bytes: 1048577, max_bytes: 1048576 },
    "la respuesta del hub ocupa más de lo que Orbit lee",
  ],
  [{ cause: "hub_response_cut", bytes: 120 }, "la respuesta del hub se interrumpió"],
  [{ cause: "hub_response_invalid" }, "la respuesta del hub no tiene el formato esperado"],
  [
    { cause: "hub_status", status: 503, code: "worker_unavailable" },
    "el hub no está disponible en este momento",
  ],
])("una petición sin confirmar explica su causa sin cifras técnicas: %j", async (data, cause) => {
  const { store, accept } = await connected();
  accept(
    status(
      [
        {
          code: "transcript_retranscribe_unconfirmed",
          op: "retranscribe-unconfirmed",
          sequence: 1,
          data,
        },
      ],
      2,
    ),
  );
  expect(store.getState().messages).toEqual([
    {
      op: "retranscribe-unconfirmed",
      class: "fixable",
      text: `El hub no ha confirmado la nueva transcripción: ${cause}. Puede seguir en curso.`,
    },
  ]);
});
