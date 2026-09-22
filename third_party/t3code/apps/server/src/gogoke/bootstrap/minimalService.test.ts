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
import { RootProfileOwnership, RootProfileOwnershipError } from "./rootProfileOwnership.ts";

const request = {
  authority: "public" as const,
  rootIdentity: "volume:0123456789abcdef/file:42",
  profileId: "current-user",
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

it("snapshots accessor-backed identity once before validation and ownership", () => {
  const makeAccessorRequest = () => {
    let rootReads = 0;
    const accessorRequest = {
      authority: "public" as const,
      get rootIdentity() {
        rootReads += 1;
        return rootReads === 1 ? "same-validated-root" : `forged-reread-${rootReads}`;
      },
      profileId: "current-user",
    };
    return { accessorRequest, rootReads: () => rootReads };
  };

  const first = makeAccessorRequest();
  const lease = constructMinimalService({
    request: first.accessorRequest,
    ownership: new RootProfileOwnership(),
    constructLocalNonModelService: () => ({ kind: "first" }),
  });
  expect(first.rootReads()).toBe(1);

  const second = makeAccessorRequest();
  expect(() =>
    constructMinimalService({
      request: second.accessorRequest,
      ownership: new RootProfileOwnership(),
      constructLocalNonModelService: () => ({ kind: "second" }),
    }),
  ).toThrow(RootProfileOwnershipError);
  expect(second.rootReads()).toBe(1);
  lease.ownership.release();
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
