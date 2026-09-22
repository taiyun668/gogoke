import { expect, it } from "@effect/vitest";
import {
  findRecipeTemplate,
  listRecipeTemplates,
  RecipeTemplateValidationError,
  validateRecipeTemplateDeclaration,
  validateRecipeTemplateRegistry,
  type RecipeTemplate,
  type RecipeTemplateDeclaration,
} from "./index.ts";

const contextSteward = findRecipeTemplate("ContextSteward");
if (!contextSteward) throw new Error("ContextSteward declaration is absent");

function mutableCopy(template: RecipeTemplateDeclaration): RecipeTemplate {
  return {
    ...template,
    modelSelector: { ...template.modelSelector },
  };
}

it("exposes exactly two disabled Antigravity declarations with an unresolved Gemini selector", () => {
  const templates = listRecipeTemplates();
  expect(templates.map((template) => template.id)).toEqual(["ContextSteward", "DreamResearcher"]);
  for (const template of templates) {
    expect(template.driver).toBe("antigravity");
    expect(template.modelSelector).toEqual({
      family: "Gemini",
      resolution: "RESOLVE_ONLY_FROM_QUALIFIED_INSTANCE",
    });
    expect(template.enabled).toBe(false);
    expect(Reflect.ownKeys(template)).toEqual([
      "id",
      "driver",
      "modelSelector",
      "toolsRef",
      "isolationRef",
      "contextRef",
      "budgetRef",
      "admissionRef",
      "enabled",
    ]);
  }
  expect(findRecipeTemplate("UnknownTemplate")).toBeNull();
});

it("returns detached snapshots whose mutation cannot change the registry", () => {
  const first = findRecipeTemplate("ContextSteward");
  if (!first) throw new Error("ContextSteward declaration is absent");
  const mutable = first as unknown as { enabled: boolean; modelSelector: { family: string } };
  mutable.modelSelector.family = "changed";
  mutable.enabled = true;
  listRecipeTemplates().pop();

  const next = findRecipeTemplate("ContextSteward");
  expect(next?.modelSelector.family).toBe("Gemini");
  expect(next?.enabled).toBe(false);
  expect(listRecipeTemplates()).toHaveLength(2);
});

it("rejects enabled, alternate drivers, literal model fields, and binding or credential fields", () => {
  expect(() => validateRecipeTemplateDeclaration({ ...contextSteward, enabled: true })).toThrow(
    RecipeTemplateValidationError,
  );
  for (const driver of ["gemini", "gemini-cli", "api"]) {
    expect(() => validateRecipeTemplateDeclaration({ ...contextSteward, driver })).toThrow(
      RecipeTemplateValidationError,
    );
  }
  expect(() =>
    validateRecipeTemplateDeclaration({
      ...contextSteward,
      modelSelector: { ...contextSteward.modelSelector, model: "gemini-2.5-pro" },
    }),
  ).toThrow(RecipeTemplateValidationError);

  for (const key of ["account", "token", "auth", "transport", "runtimeInstanceId", "modelRef"]) {
    const forbidden = { ...contextSteward, [key]: "not accepted" };
    expect(() => validateRecipeTemplateDeclaration(forbidden)).toThrow(
      RecipeTemplateValidationError,
    );
  }
});

it("rejects proxies without invoking their traps, accessors, non-plain prototypes, symbols, and extra fields", () => {
  let proxyTrapCalled = false;
  const proxied = new Proxy(contextSteward, {
    getPrototypeOf() {
      proxyTrapCalled = true;
      throw new Error("Proxy trap must not run");
    },
  });
  expect(() => validateRecipeTemplateDeclaration(proxied)).toThrow(RecipeTemplateValidationError);
  expect(proxyTrapCalled).toBe(false);

  let getterCalled = false;
  const accessor = { ...contextSteward };
  Object.defineProperty(accessor, "driver", {
    enumerable: true,
    get() {
      getterCalled = true;
      return "antigravity";
    },
  });
  expect(() => validateRecipeTemplateDeclaration(accessor)).toThrow(RecipeTemplateValidationError);
  expect(getterCalled).toBe(false);

  const inherited = Object.assign(Object.create({ inherited: true }) as object, contextSteward);
  expect(() => validateRecipeTemplateDeclaration(inherited)).toThrow(RecipeTemplateValidationError);
  expect(() =>
    validateRecipeTemplateDeclaration({ ...contextSteward, [Symbol("extra")]: true }),
  ).toThrow(RecipeTemplateValidationError);
  expect(() => validateRecipeTemplateDeclaration({ ...contextSteward, extra: "rejected" })).toThrow(
    RecipeTemplateValidationError,
  );
});

it("accepts only the closed pair and rejects duplicate or unknown template IDs", () => {
  const dreamResearcher = findRecipeTemplate("DreamResearcher");
  if (!dreamResearcher) throw new Error("DreamResearcher declaration is absent");
  expect(() =>
    validateRecipeTemplateRegistry([mutableCopy(contextSteward), mutableCopy(contextSteward)]),
  ).toThrow(RecipeTemplateValidationError);
  expect(() =>
    validateRecipeTemplateRegistry([
      mutableCopy(contextSteward),
      { ...mutableCopy(dreamResearcher), id: "Other" },
    ]),
  ).toThrow(RecipeTemplateValidationError);
  expect(() => validateRecipeTemplateDeclaration({ ...contextSteward, id: "Other" })).toThrow(
    RecipeTemplateValidationError,
  );
});

it("keeps enabled statically false", () => {
  // @ts-expect-error A recipe template can never be declared enabled.
  const enabled: RecipeTemplateDeclaration["enabled"] = true;
  expect(enabled).toBe(true);
});
