import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

export const fixtureRoot = dirname(fileURLToPath(import.meta.url));
export const desktopRoot = resolve(fixtureRoot, "../../../");

export function readDesktopSource(relativePath) {
  return readFileSync(resolve(desktopRoot, relativePath), "utf8");
}

export function readFixtureJson(name) {
  return JSON.parse(readFileSync(resolve(fixtureRoot, name), "utf8"));
}

export class FakeDependencies {
  #calls = [];

  navigation(payload) {
    this.#calls.push({ kind: "navigation", payload });
  }

  external(payload) {
    this.#calls.push({ kind: "external", payload });
  }

  microphone(payload) {
    this.#calls.push({ kind: "microphone", payload });
  }

  download(payload) {
    this.#calls.push({ kind: "download", payload });
  }

  daemon(payload) {
    this.#calls.push({ kind: "daemon", payload });
  }

  calls() {
    return [...this.#calls];
  }

  forbiddenCalls() {
    return this.#calls.filter(({ kind }) =>
      ["external", "microphone", "download", "daemon"].includes(kind),
    );
  }
}

export function assertGuardBefore(source, guard, sideEffect, label = sideEffect) {
  const guardIndex = source.indexOf(guard);
  if (guardIndex < 0) {
    throw new Error(`${label}: missing guard ${JSON.stringify(guard)}`);
  }
  const sideEffectIndex = source.indexOf(sideEffect, guardIndex + guard.length);
  if (sideEffectIndex < 0) {
    throw new Error(`${label}: missing side effect ${JSON.stringify(sideEffect)} after guard`);
  }
  if (guardIndex >= sideEffectIndex) {
    throw new Error(`${label}: guard must precede side effect`);
  }
}

export function assertNoForbiddenCalls(dependencies, label) {
  const calls = dependencies.forbiddenCalls();
  if (calls.length > 0) {
    throw new Error(`${label}: forbidden fake dependency calls: ${JSON.stringify(calls)}`);
  }
}

export function preserveModelAsset(manifest) {
  const bytes = Buffer.from(manifest.bytesBase64, "base64");
  const path = resolve("fixture-model-root", manifest.relativePath);
  return {
    bytes,
    path,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
