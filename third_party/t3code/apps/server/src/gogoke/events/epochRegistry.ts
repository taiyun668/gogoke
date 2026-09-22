import type { EventBinding } from "./types.ts";
import { snapshotBinding } from "./validation.ts";

export interface EpochLease {
  readonly binding: EventBinding;
  clear(): boolean;
}

interface Entry<Value> {
  readonly binding: EventBinding;
  readonly value: Value;
}

/** Transient connection registry with identity-based epoch compare-and-clear. */
export class EpochRegistry<Value> {
  readonly #entries = new Map<string, Entry<Value>>();

  replace(binding: EventBinding, value: Value): EpochLease {
    const currentBinding = snapshotBinding(binding);
    const entry = Object.freeze({ binding: currentBinding, value });
    this.#entries.set(currentBinding.domainId, entry);
    return Object.freeze({
      binding: currentBinding,
      clear: () => {
        if (this.#entries.get(currentBinding.domainId) !== entry) return false;
        this.#entries.delete(currentBinding.domainId);
        return true;
      },
    });
  }

  current(domainId: string): Readonly<{ binding: EventBinding; value: Value }> | null {
    const entry = this.#entries.get(domainId);
    return entry === undefined
      ? null
      : Object.freeze({ binding: entry.binding, value: entry.value });
  }
}
