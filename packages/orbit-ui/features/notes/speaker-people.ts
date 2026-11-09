import { getCopy } from "../../i18n";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { isUnnamedSpeaker } from "./notes-format";

export interface Person {
  name: string;
  subtitle: string;
}

const fold = (text: string) => text.trim().toLocaleLowerCase("es");

/** Guests suggest names; only the hub decides what a rename changes. */
export function people(note: OpenNoteSnapshot, query: string): Array<[string, Person[]]> {
  const search = fold(query);
  const match = (person: Person) => fold(`${person.name} ${person.subtitle}`).includes(search);
  const guests = (note.event?.attendees ?? [])
    .filter((guest) => !guest.is_self)
    .map((guest) => ({ name: guest.name || guest.email, subtitle: guest.email }))
    .filter(match);
  const speakers = note.speakers
    .filter((speaker) => !isUnnamedSpeaker(speaker))
    .map((speaker) => ({ name: speaker.name, subtitle: "" }))
    .filter(match);
  const groups: Array<[string, Person[]]> = [
    [getCopy().notes.guests, guests],
    [getCopy().notes.recordingSpeakers, speakers],
  ];
  return groups.filter(([, list]) => list.length > 0);
}
