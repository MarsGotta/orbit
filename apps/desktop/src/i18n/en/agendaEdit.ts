import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
// Creating, moving, editing and deleting events from the calendar.
// Kept beside `agenda.ts`, which merges it into `es.calendar`.

const readOnlyCalendar = "This calendar is read-only";
const unnamed = "Orbit can’t change this event";

export const agendaEdit = {
  write: {
    saving: "Saving…",
    deleted: "Event deleted",
    deleting: (title: string) => `Deleting “${title}”…`,
    /** `lost`: what does not come back, already joined («sin sus 3 invitados y …»). */
    undoLoses: (lost: string) => `Undo recreates it ${lost}.`,
    and: "and",
    withoutGuests: (count: number) =>
      pluralRules().select(count) === "one"
        ? "without its guest"
        : `without its ${numberFormat().format(count)} guests`,
    withoutReminders: (count: number) =>
      pluralRules().select(count) === "one"
        ? "without 1 reminder Orbit can’t edit"
        : `without ${numberFormat().format(count)} reminders Orbit can’t edit`,
    cannotUndo: (title: string) => `The last change to “${title}” can’t be undone.`,
    outsideSeries: "as a separate event outside the series",
    undoing: (title: string) => `Undoing the last change to “${title}”…`,
    nothingToUndo: "There are no changes from this session to undo.",
    // What the user can do about it; the cause stays in the log.
    unknown: "It’s unclear whether the change was saved. Check your calendar.",
    refused: "The calendar refused the change. Nothing changed.",
    // The same, for a write that ended while the calendar view was closed.
    unseenRefused: "A calendar change wasn’t saved",
    unseenUnknown: "It’s unclear whether a calendar change was saved",
    openCalendar: "Open calendar",
    dismiss: "Dismiss notice",
    blocked: {
      organizer: {
        move: "You can’t move an event organized by someone else",
        edit: "You can’t change an event organized by someone else",
        delete: "You can’t delete an event organized by someone else",
      },
      read_only: { move: readOnlyCalendar, edit: readOnlyCalendar, delete: readOnlyCalendar },
      unnamed: { move: unnamed, edit: unnamed, delete: unnamed },
    },
  },
  edit: {
    newEvent: "New event",
    title: "Title",
    when: "When",
    startDate: "Start date",
    startTime: "Start time",
    endDate: "End date",
    endTime: "End time",
    repeat: "Repeat",
    repeatOptions: {
      none: "Doesn’t repeat",
      daily: "Daily",
      weekly: "Weekly",
      monthly: "Monthly",
      yearly: "Yearly",
      kept: "Custom Mac rule",
    },
    every: "Every",
    interval: "Repeat interval",
    units: {
      daily: (count: number) => (pluralRules().select(count) === "one" ? "day" : "days"),
      weekly: (count: number) => (pluralRules().select(count) === "one" ? "week" : "weeks"),
      monthly: (count: number) => (pluralRules().select(count) === "one" ? "month" : "months"),
      yearly: (count: number) => (pluralRules().select(count) === "one" ? "year" : "years"),
    },
    locationPlaceholder: "Add location",
    notes: "Event notes",
    notesPlaceholder: "Add notes",
    notesTooLong: "These notes are too long for Orbit. Edit them in Calendar.",
    calendar: "Calendar",
    reminders: "Remind me",
    reminderOptions: [
      [0, "At the start"],
      [5, "5 min"],
      [10, "10 min"],
      [15, "15 min"],
      [30, "30 min"],
      [60, "1 h"],
      [1440, "1 day"],
    ] as const,
    minutesBefore: (minutes: number) => `${numberFormat().format(minutes)} min`,
    remindersKept: (count: number) =>
      pluralRules().select(count) === "one"
        ? "It also has 1 reminder Orbit can’t edit; it will be kept."
        : `It also has ${numberFormat().format(count)} reminders Orbit can’t edit; they will be kept.`,
    remindersTooMany: "It has more reminders than Orbit can edit. Change them in Calendar.",
    availability: "Show as",
    busy: "Busy",
    free: "Free",
    create: "Create event",
    save: "Save",
    saveThis: "This event only",
    saveFuture: "This and following events",
    delete: "Delete",
    deleteThis: "This event only",
    deleteFuture: "This and following events",
    needsTitle: "The event needs a title",
    noChanges: "No changes",
    saved: "✓ Saved to your calendar",
    changeWhen: "Change date and time",
    readOnly: {
      organizer: (name: string | null) =>
        `Organized by ${name ?? "someone else"}. Only the organizer can change it.`,
      read_only: () => `${readOnlyCalendar}. You can join and record; make changes in Calendar.`,
      unnamed: () => `${unnamed}.`,
    },
    resizeEnd: "Change end time",
    resizeFirstDay: "Change first day",
    resizeLastDay: "Change last day",
  },
} as const;
