import { afterEach, expect, it } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { configureCopy, getCopy } from "../index";
import { AgentWorkView, workSummary } from "../../features/assistant/AgentWork";
import { AIFunctions } from "../../features/settings/AIFunctions";
import type { ActivityCopy } from "../../bridge/generated/ActivityCopy";
import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { PrivacyCopy } from "../../bridge/generated/PrivacyCopy";
import { CopyProvider } from "../CopyProvider";

afterEach(cleanup);
const work: AgentWork = {
  turn_id: "t",
  started_at_ms: 0,
  tools: 1,
  subjects: ["the screen"],
  subject_codes: { "the screen": "screen" },
  items: [
    {
      id: "x",
      kind: "tool",
      phase: "started",
      steps: [],
      label: "",
      label_copy: { code: "screenshot_app", value: "Sample" },
    },
  ],
};
it.each([
  ["es", "Mirando Sample"],
  ["en", "Looking at Sample"],
] as const)("renders coded progress in %s", (language, expected) => {
  const view = render(
    <CopyProvider choice={language}>
      <AgentWorkView work={work} inFlight />
    </CopyProvider>,
  );
  expect(view.getByText(expected)).toBeTruthy();
});
it("rewrites the same live work and saved summary when the language changes", () => {
  const view = render(
    <CopyProvider choice="es">
      <AgentWorkView work={work} inFlight />
    </CopyProvider>,
  );
  expect(view.getByText("Mirando Sample")).toBeTruthy();
  expect(workSummary(work)).toBe("Usó la pantalla · 1 paso");
  view.rerender(
    <CopyProvider choice="en">
      <AgentWorkView work={work} inFlight />
    </CopyProvider>,
  );
  expect(view.getByText("Looking at Sample")).toBeTruthy();
  expect(workSummary(work)).toBe("Used the screen · 1 step");
});

const clauses: PrivacyCopy[] = [
  { code: "openrouter", material: "text", destination: "microsoft_openrouter" },
];
it.each([
  ["es", "El texto sale a Microsoft a través de OpenRouter."],
  ["en", "The text is sent to Microsoft through OpenRouter."],
] as const)("renders privacy codes in Settings in %s", (language, expected) => {
  const view = {
    settings: {
      providers: [],
      functions: [
        {
          id: "meeting_notes",
          provider_id: "operator",
          model: "sample",
          capability: "text",
          privacy: "The text is sent to Microsoft through OpenRouter.",
          privacy_copies: clauses,
        },
      ],
    },
    busy: false,
    errorPlace: null,
    error: "",
    call: async () => {},
  } as unknown as Parameters<typeof AIFunctions>[0]["view"];
  const page = render(
    <CopyProvider choice={language}>
      <AIFunctions view={view} />
    </CopyProvider>,
  );
  expect(page.getByText(expected)).toBeTruthy();
});
it("keeps fallback text for missing and future codes", () => {
  configureCopy("es", ["es"], "Deimos");
  const copy = getCopy();
  expect(copy.aiPrivacy.text(undefined, "Older hub privacy")).toBe("Older hub privacy");
  expect(copy.aiPrivacy.text([{ code: "future" }], "New privacy")).toBe("New privacy");
  expect(
    copy.agentActivity.label({ code: "future" } as unknown as ActivityCopy, "A new tool"),
  ).toBe("A new tool");
  expect(copy.agentActivity.label(undefined, "web_search")).toBe("web_search");
});
it.each(["es", "en"] as const)("renders every privacy destination in %s", (language) => {
  configureCopy(language, [language], "Deimos");
  const copy = getCopy().aiPrivacy;
  expect(copy.text([{ code: "hub", material: "audio" }], "fallback")).toBe(
    language === "es" ? "El audio se queda en tu hub." : "The audio stays on your hub.",
  );
  expect(copy.text([{ code: "network", material: "text", destination: "Cedar" }], "fallback")).toBe(
    language === "es"
      ? "El texto sale a Cedar, fuera de tu hub, por tu red o Internet."
      : "The text is sent to Cedar, outside your hub, over your network or the Internet.",
  );
  expect(copy.text([{ code: "operator" }], "fallback")).toBe(
    language === "es"
      ? "Se usa la configuración del operador; el servicio puede enviar datos fuera del hub."
      : "The operator configuration is used; the service may send data outside the hub.",
  );
});

it("keeps dynamic names as data when they resemble copy placeholders", () => {
  configureCopy("es", ["es"], "Deimos");
  expect(
    getCopy().agentActivity.label(
      { code: "screenshot_app", value: "Sample {done} $&" },
      "fallback",
    ),
  ).toBe("Mirando Sample {done} $&");
});
