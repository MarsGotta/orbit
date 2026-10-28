/** A future server key can be unknown without opening a catalogue's known
 * key set. Inherited Object properties are never copy. */
export function lookupCopy<T>(values: Readonly<Record<string, T>>, key: string): T | undefined {
  return Object.hasOwn(values, key) ? values[key] : undefined;
}
