import { createSignal } from "solid-js";

/** A value that imperative code reads back at once (`get`) and views track (`read`). Solid's own
 * signals answer a read with the last committed value until the next flush, which a command that
 * sets and then reads its own state must not see. */
export interface Box<T> {
  /** The value, untracked. */
  get(): T;
  /** The value, tracked: a memo or JSX that reads it runs again when it changes. */
  read(): T;
  set(next: T): void;
  update(transform: (value: T) => T): void;
}

export function box<T>(initial: T, equals: (a: T, b: T) => boolean = Object.is): Box<T> {
  let value = initial;
  const [version, setVersion] = createSignal(0);
  return {
    get: () => value,
    read: () => {
      version();
      return value;
    },
    set(next) {
      if (equals(value, next)) return;
      value = next;
      setVersion((n) => n + 1);
    },
    update(transform) {
      this.set(transform(value));
    },
  };
}
