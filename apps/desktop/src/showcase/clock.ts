import { NOW } from "./data/time";

/**
 * Moves this page's clock to the showcase's instant and lets it run from
 * there, so the invented day reads as today whenever the showcase is opened.
 * Only `Date` moves; timers keep their real pace.
 */
export function startShowcaseClock(): void {
  const RealDate = window.Date;
  const offset = NOW - RealDate.now();
  const shifted = () => RealDate.now() + offset;
  class ShowcaseDate extends RealDate {
    constructor(...args: ConstructorParameters<DateConstructor> | []) {
      if (args.length === 0) super(shifted());
      else super(...(args as ConstructorParameters<DateConstructor>));
    }
    static override now(): number {
      return shifted();
    }
  }
  window.Date = ShowcaseDate as DateConstructor;
}
