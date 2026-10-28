import type { spanish } from "./es";

/** Labels vary; protocol tokens, keys and parameters retain their types. */
type Tokens = "pane" | "view" | "class" | "permission" | "key" | "group";
type Rows<T> = T extends readonly (readonly [infer Key, infer Value])[]
  ? readonly (readonly [Key, CopyShape<Value>])[]
  : CopyShape<T>;
type Field<K, V> = K extends "pane"
  ? V
  : K extends "permissionRows"
    ? Rows<V>
    : K extends "command"
      ? V extends string
        ? string
        : V
      : K extends Tokens
        ? V extends string
          ? V extends Lowercase<V>
            ? V
            : string
          : CopyShape<V>
        : CopyShape<V>;
export type CopyShape<T> = T extends (...args: infer A) => infer R
  ? (...args: A) => CopyShape<R>
  : T extends string
    ? string
    : T extends readonly unknown[]
      ? { [K in keyof T]: CopyShape<T[K]> }
      : T extends object
        ? { [K in keyof T]: Field<K, T[K]> }
        : T;
export type Catalogue = CopyShape<ReturnType<typeof spanish>>;

type ExtraKeys<E, A> = [E] extends [(...args: infer _P) => infer R]
  ? [A] extends [(...args: infer _Q) => infer S]
    ? ExtraKeys<R, S>
    : never
  : [E] extends [readonly (infer R)[]]
    ? [A] extends [readonly (infer S)[]]
      ? ExtraKeys<R, S>
      : never
    : [E] extends [object]
      ? [A] extends [object]
        ? | Exclude<keyof A, keyof E>
          | {
              [K in keyof A & keyof E]: K extends "command" ? never : ExtraKeys<E[K], A[K]>;
            }[keyof A & keyof E]
        : never
      : never;

/** Checks inferred keys too, including keys introduced by topic spreads. */
export function defineCatalogue<T extends Catalogue>(
  catalogue: T &
    (ExtraKeys<Catalogue, T> extends never ? unknown : { unexpectedCatalogueKey: never }),
): T {
  return catalogue;
}
