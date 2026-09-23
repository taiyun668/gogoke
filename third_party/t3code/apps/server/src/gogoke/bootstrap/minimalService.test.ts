import { expect, it, vi } from "@effect/vitest";

import {
  constructReleaseCapability,
  DEFAULT_RUNTIME_DRIVER_IDS,
  LOCAL_NON_MODEL_CAPABILITY,
  ReleasePolicyError,
  SEALED_RELEASE_CAPABILITIES,
} from "../releasePolicy.ts";
import { constructMinimalService } from "./minimalService.ts";
import { constructGogokeService } from "./index.ts";
import {
  RootProfileOwnership,
  RootProfileOwnershipError,
  type RootProfileOwnershipRequest,
} from "./rootProfileOwnership.ts";

const request = {
  authority: "public" as const,
  rootIdentity: "volume:0123456789abcdef/file:42",
  profileId: "current-user",
};

type MinimalServiceRequestForTest = typeof request & {
  readonly requestedCapabilities?: readonly string[];
  readonly enabledRuntimeDriverIds?: readonly string[];
};

const FIXED_SEALED_CAPABILITIES = [
  "remote-access",
  "mobile-client",
  "tailscale",
  "voice",
  "update",
  "install",
  "telemetry",
  "automatic-probe",
  "title-generation",
  "account-access",
  "network-listener",
  "network-egress",
  "model-runtime",
] as const;

it("matches the fixed R4 prohibited capability set", () => {
  expect([...SEALED_RELEASE_CAPABILITIES].sort()).toEqual([...FIXED_SEALED_CAPABILITIES].sort());
});

it.each(FIXED_SEALED_CAPABILITIES)("rejects %s before its constructor is called", (capability) => {
  const constructor = vi.fn(() => ({ capability }));

  expect(() => constructReleaseCapability(capability, "test construction", constructor)).toThrow(
    ReleasePolicyError,
  );
  expect(constructor).not.toHaveBeenCalled();
});

it("constructs the local non-model service with runtime drivers disabled by default", () => {
  const ownership = new RootProfileOwnership();
  const constructor = vi.fn(() => ({ kind: LOCAL_NON_MODEL_CAPABILITY }));

  const constructed = constructMinimalService({
    request,
    ownership,
    constructLocalNonModelService: constructor,
  });

  expect(DEFAULT_RUNTIME_DRIVER_IDS).toEqual([]);
  expect(constructor).toHaveBeenCalledOnce();
  expect(constructed.service).toEqual({ kind: LOCAL_NON_MODEL_CAPABILITY });
  constructed.ownership.release();
});

it("validates the whole request before ownership mutation or local construction", () => {
  const ownership = new RootProfileOwnership();
  const forbiddenConstructor = vi.fn(() => ({ kind: "forbidden" }));

  expect(() =>
    constructMinimalService({
      request: { ...request, requestedCapabilities: [LOCAL_NON_MODEL_CAPABILITY, "telemetry"] },
      ownership,
      constructLocalNonModelService: forbiddenConstructor,
    }),
  ).toThrow(ReleasePolicyError);
  expect(forbiddenConstructor).not.toHaveBeenCalled();

  const allowedConstructor = vi.fn(() => ({ kind: "allowed" }));
  const allowed = constructMinimalService({
    request,
    ownership,
    constructLocalNonModelService: allowedConstructor,
  });
  expect(allowedConstructor).toHaveBeenCalledOnce();
  allowed.ownership.release();
});

it("rejects enabled drivers before acquiring ownership or constructing the service", () => {
  const ownership = new RootProfileOwnership();
  const constructor = vi.fn(() => ({ kind: "model" }));

  expect(() =>
    constructMinimalService({
      request: { ...request, enabledRuntimeDriverIds: ["codex"] },
      ownership,
      constructLocalNonModelService: constructor,
    }),
  ).toThrow(ReleasePolicyError);
  expect(constructor).not.toHaveBeenCalled();

  const allowed = constructMinimalService({
    request,
    ownership,
    constructLocalNonModelService: () => ({ kind: "local" }),
  });
  allowed.ownership.release();
});

it("prevents legacy and public construction from owning the same root/profile", () => {
  const legacyOwnership = new RootProfileOwnership();
  const publicOwnership = new RootProfileOwnership();
  const legacy = constructMinimalService({
    request: { ...request, authority: "legacy" },
    ownership: legacyOwnership,
    constructLocalNonModelService: () => ({ kind: "legacy" }),
  });
  const publicConstructor = vi.fn(() => ({ kind: "public" }));

  expect(() =>
    constructMinimalService({
      request,
      ownership: publicOwnership,
      constructLocalNonModelService: publicConstructor,
    }),
  ).toThrow(RootProfileOwnershipError);
  expect(publicConstructor).not.toHaveBeenCalled();

  legacy.ownership.release();
  const publicService = constructMinimalService({
    request,
    ownership: publicOwnership,
    constructLocalNonModelService: publicConstructor,
  });
  expect(publicConstructor).toHaveBeenCalledOnce();
  publicService.ownership.release();
});

it("exposes the canonical Gogoke entry with the policy fence before construction", async () => {
  await expect(
    constructGogokeService({
      request: { ...request, requestedCapabilities: ["model-runtime"] },
      root: "C:\\gogoke-fixture",
      hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
    }),
  ).rejects.toBeInstanceOf(ReleasePolicyError);
});

it("releases root/profile ownership when local construction throws", () => {
  const failingOwnership = new RootProfileOwnership();
  const retryOwnership = new RootProfileOwnership();

  expect(() =>
    constructMinimalService({
      request,
      ownership: failingOwnership,
      constructLocalNonModelService: () => {
        throw new Error("construction failed");
      },
    }),
  ).toThrow("construction failed");

  const retry = constructMinimalService({
    request,
    ownership: retryOwnership,
    constructLocalNonModelService: () => ({ kind: "retry" }),
  });
  retry.ownership.release();
});

it("releases root/profile ownership when asynchronous construction rejects", async () => {
  const failingOwnership = new RootProfileOwnership();
  const retryOwnership = new RootProfileOwnership();

  await expect(
    constructMinimalService({
      request,
      ownership: failingOwnership,
      constructLocalNonModelService: async () => {
        throw new Error("async construction failed");
      },
    }),
  ).rejects.toThrow("async construction failed");

  const retry = constructMinimalService({
    request,
    ownership: retryOwnership,
    constructLocalNonModelService: () => ({ kind: "retry" }),
  });
  retry.ownership.release();
});

it("assimilates cross-realm-style thenables and releases ownership on rejection", async () => {
  const thenable: PromiseLike<never> = {
    then: (_resolve, reject) => {
      reject?.(new Error("foreign promise rejected"));
      return thenable;
    },
  };

  await expect(
    constructMinimalService({
      request,
      ownership: new RootProfileOwnership(),
      constructLocalNonModelService: () => thenable,
    }),
  ).rejects.toThrow("foreign promise rejected");

  const retry = constructMinimalService({
    request,
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: () => ({ kind: "retry" }),
  });
  retry.ownership.release();
});

it("captures a volatile then getter once and still releases ownership on rejection", async () => {
  let thenReads = 0;
  const volatile = {
    get then() {
      thenReads += 1;
      if (thenReads > 1) return undefined;
      return (_resolve: (value: never) => void, reject: (reason: unknown) => void) => {
        reject(new Error("volatile then rejected"));
      };
    },
  };

  await expect(
    constructMinimalService({
      request,
      ownership: new RootProfileOwnership(),
      constructLocalNonModelService: () => volatile,
    }),
  ).rejects.toThrow("volatile then rejected");
  expect(thenReads).toBe(1);

  const retry = constructMinimalService({
    request,
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: () => ({ kind: "retry" }),
  });
  retry.ownership.release();
});

it("rejects accessor-backed identity without executing the accessor", () => {
  let rootReads = 0;
  const accessorRequest = {
    authority: "public" as const,
    get rootIdentity() {
      rootReads += 1;
      return "accessor-identity-root";
    },
    profileId: "current-user",
  };
  const constructor = vi.fn(() => ({ kind: "forged" }));

  expect(() =>
    constructMinimalService({
      request: accessorRequest,
      ownership: new RootProfileOwnership(),
      constructLocalNonModelService: constructor,
    }),
  ).toThrow("must be an enumerable data property");
  expect(rootReads).toBe(0);
  expect(constructor).not.toHaveBeenCalled();
});

it("rejects invalid runtime authority values", () => {
  expect(() =>
    constructMinimalService({
      request: { ...request, authority: "forged" as "public" },
      ownership: new RootProfileOwnership(),
      constructLocalNonModelService: () => ({ kind: "forged" }),
    }),
  ).toThrow("authority must be legacy or public");
});

it("rejects accessor-backed input before executing the accessor", () => {
  let requestReads = 0;
  const input = {
    get request() {
      requestReads += 1;
      return request;
    },
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: vi.fn(() => ({ kind: "forged" })),
  };

  expect(() => constructMinimalService(input)).toThrow("must be an enumerable data property");
  expect(requestReads).toBe(0);
  expect(input.constructLocalNonModelService).not.toHaveBeenCalled();
});

it("rejects extra, symbol, non-enumerable, and custom-prototype records", () => {
  const symbolKey = Symbol("forged");
  const candidates: readonly object[] = [
    { ...request, extra: true },
    { ...request, [symbolKey]: true },
    Object.defineProperty({ ...request }, "profileId", {
      value: "current-user",
      enumerable: false,
    }),
    Object.assign(Object.create({ inherited: true }), request),
  ];

  for (const candidate of candidates) {
    expect(() =>
      constructMinimalService({
        request: candidate as typeof request,
        ownership: new RootProfileOwnership(),
        constructLocalNonModelService: () => ({ kind: "forged" }),
      }),
    ).toThrow("INVALID_MINIMAL_SERVICE_INPUT");
  }

  const nullPrototypeRequest = Object.assign(Object.create(null), request) as typeof request;
  const constructed = constructMinimalService({
    request: nullPrototypeRequest,
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: () => ({ kind: "local" }),
  });
  constructed.ownership.release();
});

it("rejects Proxy input and request records without invoking traps", () => {
  const traps = vi.fn();
  const handler: ProxyHandler<object> = {
    get: () => {
      traps();
      return undefined;
    },
    getOwnPropertyDescriptor: () => {
      traps();
      return undefined;
    },
    getPrototypeOf: () => {
      traps();
      return Object.prototype;
    },
    ownKeys: () => {
      traps();
      return [];
    },
  };
  const normalInput = {
    request,
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: () => ({ kind: "forged" }),
  };

  expect(() =>
    constructMinimalService(new Proxy(normalInput, handler) as typeof normalInput),
  ).toThrow("must be a non-Proxy plain object");
  expect(traps).not.toHaveBeenCalled();

  expect(() =>
    constructMinimalService({
      ...normalInput,
      request: new Proxy(request, handler) as typeof request,
    }),
  ).toThrow("must be a non-Proxy plain object");
  expect(traps).not.toHaveBeenCalled();
});

it("rejects malformed capability arrays before ownership or construction", () => {
  const sparseArray: string[] = [];
  sparseArray.length = 1;
  const extendedArray = [LOCAL_NON_MODEL_CAPABILITY];
  Object.defineProperty(extendedArray, "extra", { value: true, enumerable: true });
  const symbolArray = [LOCAL_NON_MODEL_CAPABILITY];
  Object.defineProperty(symbolArray, Symbol("forged"), { value: true, enumerable: true });
  let accessorReads = 0;
  const accessorArray = [LOCAL_NON_MODEL_CAPABILITY];
  Object.defineProperty(accessorArray, "0", {
    enumerable: true,
    get: () => {
      accessorReads += 1;
      return LOCAL_NON_MODEL_CAPABILITY;
    },
  });
  const proxyTraps = vi.fn();
  const proxyArray = new Proxy([LOCAL_NON_MODEL_CAPABILITY], {
    get: (target, key, receiver) => {
      proxyTraps();
      return Reflect.get(target, key, receiver);
    },
  });
  const candidates: readonly unknown[] = [
    sparseArray,
    extendedArray,
    symbolArray,
    accessorArray,
    [1],
    [LOCAL_NON_MODEL_CAPABILITY, LOCAL_NON_MODEL_CAPABILITY],
    proxyArray,
  ];

  for (const field of ["requestedCapabilities", "enabledRuntimeDriverIds"] as const) {
    for (const [index, candidate] of candidates.entries()) {
      expect(() =>
        constructMinimalService({
          request: {
            ...request,
            rootIdentity: `hostile-${field}-array-root-${index}`,
            [field]: candidate as readonly string[],
          },
          ownership: new RootProfileOwnership(),
          constructLocalNonModelService: () => ({ kind: "forged" }),
        }),
      ).toThrow("INVALID_MINIMAL_SERVICE_INPUT");
    }
  }
  expect(accessorReads).toBe(0);
  expect(proxyTraps).not.toHaveBeenCalled();
});

it("passes frozen request and array snapshots to ownership", () => {
  let acquiredRequest: RootProfileOwnershipRequest | undefined;
  class CapturingOwnership extends RootProfileOwnership {
    override acquire(value: RootProfileOwnershipRequest) {
      acquiredRequest = value;
      return super.acquire(value);
    }
  }
  const requestedCapabilities = [LOCAL_NON_MODEL_CAPABILITY];
  const enabledRuntimeDriverIds: string[] = [];
  const constructed = constructMinimalService({
    request: {
      ...request,
      requestedCapabilities,
      enabledRuntimeDriverIds,
    },
    ownership: new CapturingOwnership(),
    constructLocalNonModelService: () => ({ kind: "local" }),
  });
  const captured = acquiredRequest as MinimalServiceRequestForTest | undefined;

  expect(captured).toBeDefined();
  expect(Object.isFrozen(captured)).toBe(true);
  expect(Object.isFrozen(captured?.requestedCapabilities)).toBe(true);
  expect(Object.isFrozen(captured?.enabledRuntimeDriverIds)).toBe(true);
  requestedCapabilities.push("telemetry");
  enabledRuntimeDriverIds.push("codex");
  expect(captured?.requestedCapabilities).toEqual([LOCAL_NON_MODEL_CAPABILITY]);
  expect(captured?.enabledRuntimeDriverIds).toEqual([]);
  constructed.ownership.release();
});
