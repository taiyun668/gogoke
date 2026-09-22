import { arrayData, recordData } from "../decision/engine/passive.ts";

export class EvaluationInputError extends Error {
  override readonly name = "EvaluationInputError";
  constructor() { super("EVALUATION_INPUT_REJECTED"); }
}
export const reject = (): never => { throw new EvaluationInputError(); };
export const record = (value: unknown, keys: readonly string[]) => recordData(value, keys, [], reject);
export const array = (value: unknown) => arrayData(value, reject);
export function text(value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.length > 300 || value !== value.trim()) return reject();
  return value;
}
export function ratio(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) return reject();
  return value;
}
export function optionalRatio(value: unknown): number | null { return value === null ? null : ratio(value); }
export function integer(value: unknown, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) return reject();
  return value;
}
export function revision(value: unknown): string {
  const s = text(value);
  if (!/^[1-9][0-9]*$/.test(s) || s.length > 20 || BigInt(s) > 18446744073709551615n) return reject();
  return s;
}
export function choice<const T extends string>(value: unknown, options: readonly T[]): T {
  for (const option of options) if (option === value) return option;
  return reject();
}
// Primitive framing only: never invokes caller toJSON/valueOf methods.
export function frame(values: readonly (string | number | null)[]): string {
  return values.map(value => {
    const s = value === null ? "" : String(value);
    return `${value === null ? "n" : typeof value === "string" ? "s" : "d"}${s.length}:${s}`;
  }).join("");
}
