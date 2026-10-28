import { numberFormat } from "../locale";
/** «2 h 45 min», «45 min», «4 h»: how Orbit says a length of time. */
export function duration(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${numberFormat().format(rest)} min`;
  return rest === 0
    ? `${numberFormat().format(hours)} h`
    : `${numberFormat().format(hours)} h ${numberFormat().format(rest)} min`;
}
