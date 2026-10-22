import { describe, expect, it } from "vitest";
import { genericFailure, receive } from "../../app/interceptor";
import { accessGroups } from "./agents";
import { es, globalShortcuts, spanish } from "./index";

// One thing, one name, everywhere: the words the interface uses for what it
// shows, as the design's glossary renamed them.
describe("the glossary", () => {
  it("names the home view «Hoy» and the week by its number", () => {
    expect(es.shell.briefing).toBe("Hoy");
    expect(es.calendar.weekNumber(43)).toBe("semana 43");
    expect(es.briefing.week(43)).toBe("semana 43");
  });

  it("calls the section of meetings «Reuniones», and keeps «nota» for one meeting's note", () => {
    expect(es.shell.notes).toBe("Reuniones");
    expect(es.settings.notes).toBe("Reuniones");
    expect(es.notes.empty).toBe("Aún no hay reuniones");
    expect(es.notes.emptyBody).toBe("Graba tu primera reunión y Orbit escribirá la nota por ti.");
    expect(es.meeting.openNote).toBe("Abrir nota");
    expect(es.notes.privateTab).toBe("Notas privadas");
  });

  it("records a meeting from the window, or from an event", () => {
    expect(es.meeting.meet).toBe("Grabar reunión");
    expect(es.calendar.meet).toBe("Grabar reunión");
    expect(es.meeting.recordNow).toBe("Grabar reunión");
    expect(es.meeting.startDialog).toBe("Nueva grabación");
    expect(es.meeting.record).toBe("Empezar a grabar");
    expect(es.calendar.record).toBe("Solo grabar");
  });

  it("names the conversation's agent where it answers, is asked or is called", () => {
    const copy = spanish("Atlas");
    expect(copy.shell.ask).toBe("Preguntar a Atlas");
    expect(copy.design.callAssistant).toBe("Llamar a Atlas");
    expect(copy.design.inCall).toBe("En llamada con Atlas");
    expect(copy.chat.roles.assistant).toBe("Atlas");
    expect(copy.design.collapseCall).toBe("Minimizar");
    expect(copy.design.expandCall).toBe("Abrir");
  });

  it("redoes the summary and the transcript, and says the note's steps as they go", () => {
    expect(es.noteScreen.rewrite).toBe("Rehacer resumen");
    expect(es.notes.retranscribe).toBe("Rehacer transcripción");
    expect(es.noteScreen.steps.upload.done).toBe("Audio guardado");
    expect(es.noteScreen.steps.transcript.active).toBe("Transcribiendo…");
    expect(es.noteScreen.steps.summary.waiting).toBe("Escribir el resumen");
  });

  it("stops everything, says so and offers to reactivate Orbit", () => {
    expect(es.problem.privacyStopped).toBe("Todo está parado");
    expect(es.problem.rearm).toBe("Reactivar");
    expect(es.settings.rearm).toBe("Reactivar Orbit");
    expect(es.settings.stopNow).toBe("Parar todo ahora");
    const { messages } = receive([], {
      kind: "notice",
      notice: { code: "privacy_stopped", op: "glossary-privacy" },
    });
    expect(messages[0]?.text).toBe("Todo está parado.");
    expect(messages[0]?.action?.label).toBe("Reactivar Orbit");
  });

  it("calls skills «Habilidades» and names what an agent may use by what it does", () => {
    expect(es.agents.skills).toBe("Habilidades");
    expect(es.agents.paste).toBe("Escribir desde cero");
    expect(es.agents.importFolder).toBe("Importar carpeta");
    expect(es.agents.access).toBe("Qué puede usar");
    expect(accessGroups.map(({ label }) => label)).toEqual([
      "Pantalla",
      "Usar el Mac",
      "Calendario",
      "Reuniones y notas",
      "Archivos",
    ]);
    for (const { description } of accessGroups) expect(description).not.toMatch(/orbit_/);
    expect(es.agents.kind("hermes", "Atlas")).toBe(
      "Atlas funciona con Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes.",
    );
    expect(es.agents.kind("acp", "Vega")).toBe(
      "Vega se configura desde aquí. Los cambios se guardan en el hub.",
    );
    expect(es.settings.controlHistory).toBe("Lo que ha hecho en este Mac");
  });

  it("calls the meeting's own note a personal reminder, and the open items what was left", () => {
    expect(es.briefing.reminder).toBe("Recordatorio personal");
    expect(es.briefing.reminderHelp).toBe("Solo lo ves tú, en este Mac. No va a la nota.");
    expect(es.briefing.pending).toBe("Pendiente de la última vez");
  });

  it("quits Orbit by that name, and the first run says what each skip skips", () => {
    expect(es.settings.exit).toBe("Salir de Orbit");
    expect(es.onboarding.welcome).toBe("Te damos la bienvenida a Orbit");
    expect(es.onboarding.skipAll).toBe("Configurarlo más tarde");
    expect(es.onboarding.skipStep).toBe("Hacerlo después");
  });

  it("says what could not be done, what to do, and the code to share", () => {
    expect(genericFailure("7f3a2c00-0000-4000-8000-000000000000")).toBe(
      "No se pudo completar la acción. Vuelve a intentarlo. Si se repite, comparte el código 7F3A2C con quien gestiona tu hub.",
    );
    const { messages } = receive([], { kind: "crash", op: "a1b2c3d4", code: "ui_render_failed" });
    expect(messages[0]?.text).toBe(
      "No se pudo mostrar esta vista. Vuelve a intentarlo. Si se repite, comparte el código A1B2C3 con quien gestiona tu hub.",
    );
  });

  it("writes a done copy with its check and a shortcut with macOS's symbols", () => {
    expect(es.chatActions.copied).toBe("✓ Copiado");
    expect(es.attachments.copied).toBe("✓ Copiado");
    expect(globalShortcuts.ptt).toBe("⌃⌥␣");
  });
});
