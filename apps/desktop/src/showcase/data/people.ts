import type { CalendarAttendeeSnapshot } from "../../bridge/generated/CalendarAttendeeSnapshot";

/** Invented people: Nubaria's team and Hoteles Ribera, its client. */
export const people = {
  lucia: { name: "Lucía Ferrer", email: "lucia@nubaria.example" },
  diego: { name: "Diego Salinas", email: "diego@nubaria.example" },
  marta: { name: "Marta Ibáñez", email: "marta@nubaria.example" },
  javier: { name: "Javier Ortega", email: "javier@nubaria.example" },
  irene: { name: "Irene Castaño", email: "irene@nubaria.example" },
  pablo: { name: "Pablo Medina", email: "pablo@hotelesribera.example" },
  sara: { name: "Sara Llorente", email: "sara@hotelesribera.example" },
};

type Person = keyof typeof people;

/** The guests of an event; the first one named is its organizer. */
export function guests(...names: Person[]): CalendarAttendeeSnapshot[] {
  return names.map((name) => ({
    ...people[name],
    response: name === "javier" ? "tentative" : "accepted",
    is_self: name === "lucia",
  }));
}
