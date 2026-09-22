import { expect, it, vi } from "@effect/vitest";

import type { NativeStoreConnector, NativeStoreSession } from "./nativeStoreService.ts";
import { constructNativeStoreServiceForAdapter } from "./nativeStoreService.ts";
import { RootProfileOwnership, RootProfileOwnershipError } from "./rootProfileOwnership.ts";

const request = {
  authority: "public" as const,
  rootIdentity: "volume:0123456789abcdef/file:42",
  profileId: "current-user",
};

function fixture() {
  const session: NativeStoreSession = {
    commitProject: vi.fn(async () => ({ ok: true, body: "COMMITTED", elapsedMicros: 1 })),
    readSnapshot: vi.fn(async () => ({ ok: true, body: "{}", elapsedMicros: 1 })),
    getReceipt: vi.fn(async () => ({ ok: true, body: "{}", elapsedMicros: 1 })),
    commitContextVersion: vi.fn(async () => {
      throw new Error("unused");
    }),
    reserve: vi.fn(async () => {
      throw new Error("unused");
    }),
    begin: vi.fn(async () => {
      throw new Error("unused");
    }),
    recordDispatchOutcome: vi.fn(async () => {
      throw new Error("unused");
    }),
    publishDecisionSnapshot: vi.fn(async () => {
      throw new Error("unused");
    }),
    commitDecision: vi.fn(async () => {
      throw new Error("unused");
    }),
    readDecisionReplay: vi.fn(async () => {
      throw new Error("unused");
    }),
    publishContextAssemblySnapshot: vi.fn(async () => {
      throw new Error("unused");
    }),
    commitTaskContextRequirements: vi.fn(async () => {
      throw new Error("unused");
    }),
    readTaskContextRequirements: vi.fn(async () => {
      throw new Error("unused");
    }),
    readContextAssemblyBasis: vi.fn(async () => {
      throw new Error("unused");
    }),
    listContextAssemblySources: vi.fn(async () => {
      throw new Error("unused");
    }),
    readGranteeContextSet: vi.fn(async () => {
      throw new Error("unused");
    }),
    commitContextManifest: vi.fn(async () => {
      throw new Error("unused");
    }),
    readContextManifest: vi.fn(async () => {
      throw new Error("unused");
    }),
    close: vi.fn(async () => undefined),
  };
  const connector: NativeStoreConnector = { attach: vi.fn(async () => session) };
  return { connector, session };
}

it("constructs only the fixed native-store path after release validation", async () => {
  const { connector, session } = fixture();
  const service = await constructNativeStoreServiceForAdapter({
    request,
    root: "C:\\gogoke-fixture",
    hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
    ownership: new RootProfileOwnership(),
    connector,
  });
  expect(connector.attach).toHaveBeenCalledWith({
    root: "C:\\gogoke-fixture",
    hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
  });
  expect(service.store).toBe(session);
  await service.close();
  expect(session.close).toHaveBeenCalledOnce();
});

it("rejects forbidden capability and invalid paths before attaching", async () => {
  const { connector } = fixture();
  await expect(
    constructNativeStoreServiceForAdapter({
      request: { ...request, requestedCapabilities: ["network-listener"] },
      root: "relative-root",
      hostBinary: "relative-host.exe",
      ownership: new RootProfileOwnership(),
      connector,
    }),
  ).rejects.toThrow();
  expect(connector.attach).not.toHaveBeenCalled();
});

it("retains ownership when native close is unknown and releases only after confirmed close", async () => {
  const ownership = new RootProfileOwnership();
  const { connector, session } = fixture();
  vi.mocked(session.close).mockRejectedValueOnce(new Error("close unknown"));
  const service = await constructNativeStoreServiceForAdapter({
    request,
    root: "C:\\gogoke-fixture",
    hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
    ownership,
    connector,
  });
  await expect(service.close()).rejects.toThrow("close unknown");

  const second = fixture();
  await expect(
    constructNativeStoreServiceForAdapter({
      request,
      root: "C:\\gogoke-fixture",
      hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
      ownership,
      connector: second.connector,
    }),
  ).rejects.toBeInstanceOf(RootProfileOwnershipError);

  await service.close();
  const replacement = await constructNativeStoreServiceForAdapter({
    request,
    root: "C:\\gogoke-fixture",
    hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
    ownership,
    connector: second.connector,
  });
  await replacement.close();
});

it("snapshots the connector method and path inputs before asynchronous construction", async () => {
  const { connector } = fixture();
  let rootReads = 0;
  let attachReads = 0;
  const volatileConnector = {
    get attach() {
      attachReads += 1;
      return connector.attach;
    },
  };
  const input = {
    request,
    get root() {
      rootReads += 1;
      return rootReads === 1 ? "C:\\gogoke-fixture" : "relative-forged";
    },
    hostBinary: "C:\\gogoke-bin\\gogoke-native-host.exe",
    ownership: new RootProfileOwnership(),
    connector: volatileConnector,
  };
  await expect(constructNativeStoreServiceForAdapter(input)).rejects.toThrow(
    "input.root must be an enumerable data property",
  );
  expect(rootReads).toBe(0);
  expect(attachReads).toBe(0);
});
