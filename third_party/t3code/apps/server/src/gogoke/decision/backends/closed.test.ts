import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import type { DecisionBackendInput } from "./local.ts";
import {
  CLOSED_GENERATIVE_BACKEND,
  CLOSED_JEV_BACKEND,
  createExternalDecisionBackend,
  DecisionBackendClosedError,
} from "./closed.ts";

// Source-import policy is confirmed by read-only review; this suite only observes the callable boundary.
function hostileObject(touches: string[], label: string): object {
  const touch = (trap: string): never => {
    touches.push(`${label}.${trap}`);
    throw new Error(`${label} ${trap} trap was touched`);
  };
  return new Proxy(Object.create(null) as object, {
    get() {
      return touch("get");
    },
    set() {
      return touch("set");
    },
    has() {
      return touch("has");
    },
    ownKeys() {
      return touch("ownKeys");
    },
    getOwnPropertyDescriptor() {
      return touch("getOwnPropertyDescriptor");
    },
    defineProperty() {
      return touch("defineProperty");
    },
    deleteProperty() {
      return touch("deleteProperty");
    },
    getPrototypeOf() {
      return touch("getPrototypeOf");
    },
    setPrototypeOf() {
      return touch("setPrototypeOf");
    },
    isExtensible() {
      return touch("isExtensible");
    },
    preventExtensions() {
      return touch("preventExtensions");
    },
  });
}

function isNotQualified(error: unknown): error is DecisionBackendClosedError {
  return (
    error instanceof DecisionBackendClosedError &&
    error.name === "DecisionBackendClosedError" &&
    error.code === "NOT_QUALIFIED" &&
    error.message === "NOT_QUALIFIED"
  );
}

describe("closed external Decision backends", () => {
  it("rejects construction before observing configuration or thenables", () => {
    const touches: string[] = [];
    const configuration = hostileObject(touches, "configuration");
    const thenable = Object.defineProperty({}, "then", {
      get() {
        touches.push("then");
        throw new Error("configuration assimilated");
      },
    });
    const getterConfiguration = Object.defineProperty(Object.create(null), "apiKey", {
      get() {
        touches.push("apiKey");
        throw new Error("configuration getter invoked");
      },
    });

    Assert.throws(() => createExternalDecisionBackend(configuration), isNotQualified);
    Assert.throws(() => createExternalDecisionBackend(thenable), isNotQualified);
    Assert.throws(() => createExternalDecisionBackend(getterConfiguration), isNotQualified);
    Assert.deepEqual(touches, []);
  });

  it("rejects JEV and GENERATIVE calls without observing arguments or thenables", async () => {
    const touches: string[] = [];
    const input = hostileObject(touches, "input");
    const thenable = Object.defineProperty({}, "then", {
      get() {
        touches.push("then");
        throw new Error("input assimilated");
      },
    });
    const getterInput = Object.defineProperty(Object.create(null), "scenario", {
      get() {
        touches.push("scenario");
        throw new Error("input getter invoked");
      },
    });

    for (const backend of [CLOSED_JEV_BACKEND, CLOSED_GENERATIVE_BACKEND]) {
      Assert.equal(backend.kind, backend === CLOSED_JEV_BACKEND ? "JEV" : "GENERATIVE");
      await Assert.rejects(
        backend.evaluate(input as unknown as DecisionBackendInput),
        isNotQualified,
      );
      await Assert.rejects(
        backend.evaluate(thenable as unknown as DecisionBackendInput),
        isNotQualified,
      );
      await Assert.rejects(
        backend.evaluate(getterInput as unknown as DecisionBackendInput),
        isNotQualified,
      );
    }
    Assert.deepEqual(touches, []);
  });
});
