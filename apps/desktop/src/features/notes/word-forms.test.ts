import { describe, expect, it } from "vitest";
import { corrected, fixedPlaces, occurrences, wordAt } from "./word-forms";

describe("the word under the pointer", () => {
  const text = "Ya está en Orvit. Lo probamos";

  it("is the whole run of letters around the character, accents included", () => {
    expect(wordAt(text, 13)).toEqual({ start: 11, end: 16 });
    expect(wordAt(text, 4)).toEqual({ start: 3, end: 7 });
  });

  it("is none on a space, a sign or past the end", () => {
    expect(wordAt(text, 2)).toBeNull();
    expect(wordAt(text, 16)).toBeNull();
    expect(wordAt(text, text.length)).toBeNull();
  });
});

describe("a heard form's places in the meeting, as the hub's dictionaries find them", () => {
  it("counts it whole and in any case", () => {
    expect(occurrences("Orvit, orvit y ORVIT.", "Orvit")).toHaveLength(3);
  });

  it("does not count it inside another word", () => {
    expect(occurrences("Orvitas y laorvit", "orvit")).toEqual([]);
  });

  it("finds a phrase, and nothing for an empty form", () => {
    expect(occurrences("con Hoteles rivera mañana", "hoteles Rivera")).toEqual([
      { start: 4, end: 18 },
    ]);
    expect(occurrences("algo", "  ")).toEqual([]);
  });

  it("takes signs in the form literally", () => {
    expect(occurrences("Empezamos. Empezamos", "Empezamos.")).toEqual([{ start: 0, end: 10 }]);
  });
});

describe("a correction in the transcript", () => {
  it("reads each place as the correct word until the hub's read brings it", () => {
    expect(corrected("Orvit y orvit, ya", "Orvit", "Órbita")).toBe("Órbita y Órbita, ya");
  });

  it("marks where the corrected words sit, each later one moved by the earlier ones", () => {
    const turns = [
      { index: 0, text: "Orvit y Orvit" },
      { index: 1, text: "Nada" },
      { index: 2, text: "Lo dice orvit" },
    ];
    expect(fixedPlaces(turns, "Orvit", "Órbita")).toEqual([
      { turn: 0, start: 0, end: 6 },
      { turn: 0, start: 9, end: 15 },
      { turn: 2, start: 8, end: 14 },
    ]);
    expect(corrected(turns[0]!.text, "Orvit", "Órbita").slice(9, 15)).toBe("Órbita");
  });
});
