/**
 * Geometry of the calendar grid (P9-20). Pure: minutes,
 * days and pixels in, positions out.
 */

/** About twelve hours fit on screen; an hour is never shorter than this. */
const MIN_HOUR_PX = 61;
/** What the header, the day row and the all-day lane take above the hours. */
const CHROME_PX = 113;
const VISIBLE_HOURS = 12;
export const MIN_EVENT_HEIGHT = 22;
/** Gap under a box so two consecutive events do not touch. */
const EVENT_GAP_PX = 3;
/** An event shorter than this is placed as if it lasted this long. */
const MIN_PLACED_MINUTES = 10;
/** An event this long after its neighbour overlays it instead of splitting. */
export const CASCADE_AFTER_MINUTES = 45;
/** How far a cascaded event is indented over the one under it. */
export const CASCADE_INDENT_PERCENT = 5;

export function hourHeight(visibleHeight: number, zoom: number): number {
  return Math.max((visibleHeight - CHROME_PX) / VISIBLE_HOURS, MIN_HOUR_PX) * zoom;
}

export function calculateEventBox(startMinutes: number, endMinutes: number, hourPx: number) {
  return {
    height: Math.max(((endMinutes - startMinutes) / 60) * hourPx - EVENT_GAP_PX, MIN_EVENT_HEIGHT),
    top: (startMinutes / 60) * hourPx,
  };
}

export interface TimedEvent {
  endMinutes: number;
  id: string;
  startMinutes: number;
}

export interface PlacedEvent {
  /** How many events this one is cascaded over. */
  depth: number;
  id: string;
  leftPercent: number;
  widthPercent: number;
}

interface Lane {
  depth: number;
  end: number;
  event: TimedEvent;
  lane: number;
}

/**
 * Columns by start time. An event that starts close to the ones it overlaps
 * takes the first free column and the group shares the width; one that
 * starts `CASCADE_AFTER_MINUTES` or more after the latest of them overlays
 * it, indented, and keeps its width.
 */
export function layoutDayEvents(events: readonly TimedEvent[]): PlacedEvent[] {
  const ordered = [...events].sort(
    (first, second) =>
      first.startMinutes - second.startMinutes || second.endMinutes - first.endMinutes,
  );
  const placed = new Map<string, PlacedEvent>();
  let group: Lane[] = [];
  const commit = () => {
    const lanes = Math.max(1, ...group.map((item) => item.lane + 1));
    const width = 100 / lanes;
    for (const item of group) {
      const indent = item.depth * CASCADE_INDENT_PERCENT;
      placed.set(item.event.id, {
        depth: item.depth,
        id: item.event.id,
        leftPercent: item.lane * width + indent,
        widthPercent: width - indent,
      });
    }
    group = [];
  };
  for (const event of ordered) {
    const end = Math.max(event.endMinutes, event.startMinutes + MIN_PLACED_MINUTES);
    const active = group.filter((item) => item.end > event.startMinutes);
    if (active.length === 0) commit();
    group.push(place(event, end, active));
  }
  commit();
  return events.map(
    (event) =>
      placed.get(event.id) ?? { depth: 0, id: event.id, leftPercent: 0, widthPercent: 100 },
  );
}

function place(event: TimedEvent, end: number, active: Lane[]): Lane {
  const latest = active.reduce<Lane | undefined>(
    (found, item) =>
      found === undefined || item.event.startMinutes >= found.event.startMinutes ? item : found,
    undefined,
  );
  if (latest && event.startMinutes - latest.event.startMinutes >= CASCADE_AFTER_MINUTES) {
    return { depth: latest.depth + 1, end, event, lane: latest.lane };
  }
  const taken = new Set(active.map((item) => item.lane));
  let lane = 0;
  while (taken.has(lane)) lane += 1;
  return { depth: 0, end, event, lane };
}

export interface AllDayItem {
  firstDay: number;
  id: string;
  lastDay: number;
}

export interface AllDayBar {
  clippedEnd: boolean;
  clippedStart: boolean;
  column: number;
  id: string;
  row: number;
  span: number;
}

/**
 * Bars across the visible columns (civil days, ascending; weekends may be
 * missing). A bar cut by the view says on which side, so its title can stay
 * in sight. Rows are filled greedily by start, longest first, first row first.
 */
export function allDayLayout(items: readonly AllDayItem[], columns: readonly number[]) {
  const rowEnds: number[] = [];
  const bars: AllDayBar[] = [];
  const ordered = [...items].sort(
    (first, second) => first.firstDay - second.firstDay || second.lastDay - first.lastDay,
  );
  for (const item of ordered) {
    const visible = columns
      .map((day, index) => ({ day, index }))
      .filter(({ day }) => day >= item.firstDay && day <= item.lastDay);
    const first = visible[0];
    const last = visible.at(-1);
    if (!first || !last) continue;
    let row = rowEnds.findIndex((end) => end < first.index);
    if (row === -1) row = rowEnds.length;
    rowEnds[row] = last.index;
    bars.push({
      clippedEnd: item.lastDay > last.day,
      clippedStart: item.firstDay < first.day,
      column: first.index,
      id: item.id,
      row,
      span: last.index - first.index + 1,
    });
  }
  return bars;
}
