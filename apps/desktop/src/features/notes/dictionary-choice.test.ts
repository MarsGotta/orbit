import { describe, expect, it } from "vitest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import {
  chosen,
  correctionOrder,
  dictionaryOptions,
  holds,
  meetingDictionaries,
  orderTarget,
  released,
  undoOrder,
} from "./dictionary-choice";

const listing = {
  dictionary_id: "dic-albar",
  dictionary_ids: ["dic-albar", "dic-lumo"],
  source: "requested",
  series: null,
};
const meeting = (dictionary?: OpenNoteSnapshot["dictionary"]) =>
  ({ recording_id: "rec-taller", dictionary }) as OpenNoteSnapshot;
const older = meeting({ dictionary_id: "dic-albar", source: "requested", series: null });

describe("the dictionaries a meeting uses", () => {
  it("are the hub's list, or an older hub's one as a list of one", () => {
    expect(meetingDictionaries(meeting(listing))).toEqual(["dic-albar", "dic-lumo"]);
    expect(meetingDictionaries(older)).toEqual(["dic-albar"]);
    expect(
      meetingDictionaries(meeting({ dictionary_id: null, source: null, series: null })),
    ).toEqual([]);
  });

  it("are unknown, not none, before the hub was read", () => {
    expect(meetingDictionaries(meeting())).toBeNull();
  });

  it("can be several only with a hub that takes lists", () => {
    expect(chosen(meeting(listing), ["dic-albar"], ["dic-albar", "dic-lumo"])).toEqual([
      "dic-albar",
      "dic-lumo",
    ]);
    expect(chosen(older, ["dic-albar"], ["dic-albar", "dic-lumo"])).toEqual(["dic-lumo"]);
    expect(chosen(older, ["dic-albar"], [])).toEqual([]);
  });
});

describe("a correction's orders", () => {
  it("go into every chosen dictionary at once to a hub that takes lists", () => {
    const order = correctionOrder(
      meeting(listing),
      ["dic-albar", "dic-nuevo"],
      "orvit",
      "Orbit",
      "Nuevo",
    );
    expect(order).toEqual({
      action: "correct",
      recording_id: "rec-taller",
      dictionary_ids: ["dic-albar", "dic-nuevo"],
      wrong: "orvit",
      correct: "Orbit",
      name: "Nuevo",
    });
    expect(orderTarget(order)).toBe("dictionaries:dic-albar,dic-nuevo");
  });

  it("name one dictionary to a hub up to 0.10.1", () => {
    const order = correctionOrder(older, ["dic-albar"], "orvit", "Orbit", null);
    expect(order).toEqual({
      action: "correct",
      recording_id: "rec-taller",
      dictionary_id: "dic-albar",
      wrong: "orvit",
      correct: "Orbit",
      name: null,
    });
    expect(orderTarget(order)).toBe("dictionary:dic-albar");
  });

  it("are undone in every dictionary under the correction's own target", () => {
    const undo = undoOrder(["dic-albar", "dic-lumo"], "orvit", "Orbit");
    expect(undo).toEqual({
      action: "remove_correction",
      dictionary_ids: ["dic-albar", "dic-lumo"],
      wrong: "orvit",
      correct: "Orbit",
    });
    expect(orderTarget(undo)).toBe("dictionaries:dic-albar,dic-lumo");
  });
});

const library = {
  dictionary_items: [
    {
      dictionary: {
        id: "dic-albar",
        name: "Ferretería Albar",
        terms: [{ id: "t1", correct: "Orbit", wrong: ["orvit"] }],
        series: [
          { calendar_item_id: "a", title: "Revisión semanal" },
          { calendar_item_id: "b", title: "Comité" },
        ],
      },
      pending: false,
    },
    {
      dictionary: { id: "dic-lumo", name: "Lumo", terms: [], series: [] },
      pending: false,
    },
  ],
} as unknown as NoteLibrarySnapshot;

describe("what the library says of a correction", () => {
  it("holds it only once every dictionary has it, in any case of the heard form", () => {
    expect(holds(library, ["dic-albar"], "Orvit", "Orbit")).toBe(true);
    expect(holds(library, ["dic-albar", "dic-lumo"], "Orvit", "Orbit")).toBe(false);
    expect(released(library, ["dic-lumo"], "Orvit", "Orbit")).toBe(true);
    expect(released(library, ["dic-albar", "dic-lumo"], "Orvit", "Orbit")).toBe(false);
  });

  it("offers each dictionary with what uses it, its words and «esta reunión»", () => {
    expect(dictionaryOptions(library, ["dic-albar"])).toEqual([
      {
        id: "dic-albar",
        name: "Ferretería Albar",
        detail: "Revisión semanal y Comité · 1 palabra",
        tag: "esta reunión",
      },
      { id: "dic-lumo", name: "Lumo", detail: "Ninguna serie · 0 palabras", tag: undefined },
    ]);
  });
});
