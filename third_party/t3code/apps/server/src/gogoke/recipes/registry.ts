import { isProxy } from "node:util/types";

export const RECIPE_TEMPLATE_IDS = Object.freeze(["ContextSteward", "DreamResearcher"] as const);

export type RecipeTemplateId = (typeof RECIPE_TEMPLATE_IDS)[number];

export interface RecipeTemplateModelSelectorDeclaration {
  readonly family: "Gemini";
  readonly resolution: "RESOLVE_ONLY_FROM_QUALIFIED_INSTANCE";
}

/** A passive declaration; it is not an ExecutionRecipe and cannot start work. */
export interface RecipeTemplateDeclaration {
  readonly id: RecipeTemplateId;
  readonly driver: "antigravity";
  readonly modelSelector: RecipeTemplateModelSelectorDeclaration;
  readonly toolsRef: string;
  readonly isolationRef: string;
  readonly contextRef: string;
  readonly budgetRef: string;
  readonly admissionRef: string;
  readonly enabled: false;
}

export type RecipeTemplate = RecipeTemplateDeclaration;

export class RecipeTemplateValidationError extends Error {
  override readonly name = "RecipeTemplateValidationError";
  readonly code = "INVALID_RECIPE_TEMPLATE_DECLARATION";

  constructor() {
    super("invalid recipe template declaration");
  }
}

type Reject = () => never;

const getPrototypeOf = Object.getPrototypeOf;
const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
const ownKeys = Reflect.ownKeys;
const hasOwn = Object.hasOwn;
const isArray = Array.isArray;
const createObject = Object.create;

const declarationKeys = [
  "id",
  "driver",
  "modelSelector",
  "toolsRef",
  "isolationRef",
  "contextRef",
  "budgetRef",
  "admissionRef",
  "enabled",
] as const;
const modelSelectorKeys = ["family", "resolution"] as const;

function invalid(): never {
  throw new RecipeTemplateValidationError();
}

function ownRecordSnapshot(
  value: unknown,
  keysRequired: readonly string[],
): Readonly<Record<string, unknown>> {
  if (value === null || typeof value !== "object" || isProxy(value) || isArray(value))
    return invalid();
  const prototype = getPrototypeOf(value);
  if (prototype !== null && prototype !== Object.prototype) return invalid();

  const keys = ownKeys(value);
  if (keys.length !== keysRequired.length) return invalid();
  const snapshot = createObject(null) as Record<string, unknown>;
  for (const key of keys) {
    if (typeof key !== "string" || !keysRequired.includes(key)) return invalid();
    const descriptor = getOwnPropertyDescriptor(value, key);
    if (!descriptor || !hasOwn(descriptor, "value") || descriptor.enumerable !== true)
      return invalid();
    snapshot[key] = descriptor.value;
  }
  for (const key of keysRequired) if (!hasOwn(snapshot, key)) return invalid();
  return snapshot;
}

function ownTwoEntryArraySnapshot(value: unknown): readonly unknown[] {
  if (value === null || typeof value !== "object" || isProxy(value) || !isArray(value))
    return invalid();
  if (getPrototypeOf(value) !== Array.prototype) return invalid();

  const lengthDescriptor = getOwnPropertyDescriptor(value, "length");
  if (!lengthDescriptor || !hasOwn(lengthDescriptor, "value") || lengthDescriptor.value !== 2)
    return invalid();
  const keys = ownKeys(value);
  if (keys.length !== 3) return invalid();
  for (const key of keys) {
    if (key === "length") continue;
    if (typeof key !== "string" || !/^(0|1)$/u.test(key)) return invalid();
  }

  const snapshot: unknown[] = [];
  for (let index = 0; index < 2; index++) {
    const descriptor = getOwnPropertyDescriptor(value, String(index));
    if (!descriptor || !hasOwn(descriptor, "value") || descriptor.enumerable !== true)
      return invalid();
    snapshot[index] = descriptor.value;
  }
  return snapshot;
}

function isRecipeTemplateId(value: unknown): value is RecipeTemplateId {
  return value === "ContextSteward" || value === "DreamResearcher";
}

function declarativeRef(id: RecipeTemplateId, slot: string): string {
  return `recipe-template:${id}:${slot}`;
}

/** Validate and copy a declaration without invoking accessors or Proxy traps. */
export function validateRecipeTemplateDeclaration(input: unknown): RecipeTemplateDeclaration {
  const declaration = ownRecordSnapshot(input, declarationKeys);
  if (!isRecipeTemplateId(declaration.id)) return invalid();
  const id = declaration.id;
  if (declaration.driver !== "antigravity") return invalid();

  const selector = ownRecordSnapshot(declaration.modelSelector, modelSelectorKeys);
  if (
    selector.family !== "Gemini" ||
    selector.resolution !== "RESOLVE_ONLY_FROM_QUALIFIED_INSTANCE"
  ) {
    return invalid();
  }

  if (
    declaration.toolsRef !== declarativeRef(id, "tools") ||
    declaration.isolationRef !== declarativeRef(id, "isolation") ||
    declaration.contextRef !== declarativeRef(id, "context") ||
    declaration.budgetRef !== declarativeRef(id, "budget") ||
    declaration.admissionRef !== declarativeRef(id, "admission")
  ) {
    return invalid();
  }
  if (declaration.enabled !== false) return invalid();
  return {
    id,
    driver: "antigravity",
    modelSelector: {
      family: "Gemini",
      resolution: "RESOLVE_ONLY_FROM_QUALIFIED_INSTANCE",
    },
    toolsRef: declarativeRef(id, "tools"),
    isolationRef: declarativeRef(id, "isolation"),
    contextRef: declarativeRef(id, "context"),
    budgetRef: declarativeRef(id, "budget"),
    admissionRef: declarativeRef(id, "admission"),
    enabled: false,
  };
}

/** Validate the closed two-template set and return detached declaration snapshots. */
export function validateRecipeTemplateRegistry(input: unknown): RecipeTemplate[] {
  const entries = ownTwoEntryArraySnapshot(input);
  const snapshots: RecipeTemplate[] = [];
  const seen = new Set<RecipeTemplateId>();
  for (let index = 0; index < entries.length; index++) {
    const entry = validateRecipeTemplateDeclaration(entries[index]);
    if (seen.has(entry.id)) return invalid();
    seen.add(entry.id);
    snapshots[index] = entry;
  }
  if (!seen.has("ContextSteward") || !seen.has("DreamResearcher")) return invalid();
  return snapshots;
}

function templateDeclaration(id: RecipeTemplateId): RecipeTemplateDeclaration {
  return {
    id,
    driver: "antigravity",
    modelSelector: {
      family: "Gemini",
      resolution: "RESOLVE_ONLY_FROM_QUALIFIED_INSTANCE",
    },
    toolsRef: declarativeRef(id, "tools"),
    isolationRef: declarativeRef(id, "isolation"),
    contextRef: declarativeRef(id, "context"),
    budgetRef: declarativeRef(id, "budget"),
    admissionRef: declarativeRef(id, "admission"),
    enabled: false,
  };
}

function freezeInternal(entry: RecipeTemplateDeclaration): RecipeTemplateDeclaration {
  return Object.freeze({ ...entry, modelSelector: Object.freeze({ ...entry.modelSelector }) });
}

const initialEntries = validateRecipeTemplateRegistry([
  templateDeclaration("ContextSteward"),
  templateDeclaration("DreamResearcher"),
]);
const registry = new Map<RecipeTemplateId, RecipeTemplateDeclaration>();
for (const entry of initialEntries) registry.set(entry.id, freezeInternal(entry));

function detachedCopy(entry: RecipeTemplateDeclaration): RecipeTemplate {
  return {
    id: entry.id,
    driver: entry.driver,
    modelSelector: { ...entry.modelSelector },
    toolsRef: entry.toolsRef,
    isolationRef: entry.isolationRef,
    contextRef: entry.contextRef,
    budgetRef: entry.budgetRef,
    admissionRef: entry.admissionRef,
    enabled: false,
  };
}

export function listRecipeTemplates(): RecipeTemplate[] {
  const result: RecipeTemplate[] = [];
  for (const id of RECIPE_TEMPLATE_IDS) {
    const entry = registry.get(id);
    if (!entry) return invalid();
    result.push(detachedCopy(entry));
  }
  return result;
}

/** Unknown IDs are explicitly absent; the returned declaration is detached. */
export function findRecipeTemplate(id: string): RecipeTemplate | null {
  if (typeof id !== "string" || !isRecipeTemplateId(id)) return null;
  const entry = registry.get(id);
  return entry ? detachedCopy(entry) : null;
}
