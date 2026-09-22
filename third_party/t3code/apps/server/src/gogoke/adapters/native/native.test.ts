import { expect, it } from "@effect/vitest";

import {
  NativeAdapterError,
  NativeAdapterRegistry,
  createFakeProcessPort,
  createFakeSdkTransport,
  computeMaterialDigest,
  computeSourceLedgerDigest,
  cloneFrame,
  dataFrame,
  eofFrame,
  errorFrame,
  openNativeAdapterSession,
  openManagedProcessPort,
  openManagedSdkPort,
  sanitizeEnvironment,
  validateLaunchSpec,
  validateSourceContract,
  type NativeAdapterBinding,
  type NativeAdapterSourceContractInput,
  type NativeOperationContext,
  type ActiveBytePort,
  type ManagedProcessPort,
} from "./index.ts";

const binding: NativeAdapterBinding = {
  runtimeInstanceId: "runtime-one",
  profileId: "fixture-profile",
  authRevision: "1",
  generation: "0",
  scope: "public",
};

const sourceMaterials = [
  { name: "source:src/adapter.ts", bytes: [1, 2, 3] },
  { name: "artifact:dist/adapter.mjs", bytes: [4, 5] },
  { name: "license:LICENSE.txt", bytes: [6] },
  { name: "nodeBundle:lockfile", bytes: [7] },
  { name: "nodeBundle:bundle", bytes: [8] },
  { name: "dependency:fixture-runtime@1.0.0", bytes: [9] },
  { name: "asset:LICENSE.txt", bytes: [10] },
  { name: "dependencyNotice:fixture-runtime@1.0.0:NOTICE.txt", bytes: [11] },
] as const;

const source: NativeAdapterSourceContractInput = {
  schema: "gogoke.native-adapter-source.v1",
  adapterId: "fixture-driver",
  adapterVersion: "1",
  source: {
    repository: "https://example.invalid/fixture-driver",
    revision: "0123456789abcdef0123456789abcdef01234567",
    paths: ["src/adapter.ts"],
    digest: computeSourceLedgerDigest(["src/adapter.ts"], sourceMaterials),
  },
  artifact: {
    entrypoint: "dist/adapter.mjs",
    digest: computeMaterialDigest([4, 5]),
  },
  license: {
    spdxId: "MIT",
    noticeRef: "LICENSE.txt",
    digest: computeMaterialDigest([6]),
  },
  nodeBundle: {
    nodeVersion: "24.13.1",
    packageManager: "pnpm@11.10.0",
    lockfileDigest: computeMaterialDigest([7]),
    bundleDigest: computeMaterialDigest([8]),
    distributed: false,
  },
  dependencies: [
    {
      name: "fixture-runtime",
      version: "1.0.0",
      digest: computeMaterialDigest([9]),
      spdxId: "MIT",
      noticeRef: "NOTICE.txt",
      noticeDigest: computeMaterialDigest([11]),
    },
  ],
  assets: [
    {
      path: "LICENSE.txt",
      digest: computeMaterialDigest([10]),
      role: "notice",
    },
  ],
  isolation: {
    profile: "managed-private-root",
    inheritedEnvironment: "allowlist",
    network: "disabled",
    credentials: "none",
    userHome: "not-inherited",
  },
};

function context(
  value: NativeAdapterBinding = binding,
  deadlineAt = Date.now() + 5_000,
  signal: AbortSignal = new AbortController().signal,
): NativeOperationContext {
  return { signal, deadlineAt, binding: value, custodyRef: "custody-fixture" };
}

function registry(): NativeAdapterRegistry {
  return new NativeAdapterRegistry([
    {
      driverId: "fixture-driver",
      adapterVersion: "1",
      source,
      sourceMaterials,
      transports: ["process", "sdk"],
    },
  ]);
}

function launch(value: NativeAdapterBinding = binding) {
  return {
    adapterId: "fixture-driver",
    adapterVersion: "1",
    binding: value,
    custodyRef: "custody-fixture",
    executableRef: "fixtures/fake-driver.mjs",
    cwdRef: "fixtures/root",
    environment: { PATH: "fixtures/bin", API_TOKEN: "must-not-be-read" },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolveValue, rejectValue) => {
    resolve = resolveValue;
    reject = rejectValue;
  });
  return { promise, resolve, reject };
}

it("keeps source, license and artifact hashes explicit and immutable", () => {
  const normalized = validateSourceContract(source, sourceMaterials);
  expect(Object.isFrozen(normalized)).toBe(true);
  expect(normalized.materials).toHaveLength(8);
  expect(Object.isFrozen(normalized.materials[0]?.bytes)).toBe(true);
  expect(Object.isFrozen(normalized.license)).toBe(true);
  expect(normalized.license.spdxId).toBe("MIT");
  expect(() => {
    (normalized.license as { spdxId: string }).spdxId = "Apache-2.0";
  }).toThrow();
  expect(() =>
    validateSourceContract({ ...source, license: undefined as never }, sourceMaterials),
  ).toThrowError(NativeAdapterError);
  expect(() =>
    validateSourceContract(
      { ...source, nodeBundle: { ...source.nodeBundle, distributed: true as never } },
      sourceMaterials,
    ),
  ).toThrow("distributed");
  expect(() =>
    validateSourceContract(
      { ...source, artifact: { ...source.artifact, digest: computeMaterialDigest([99]) } },
      sourceMaterials,
    ),
  ).toThrow("material bytes");
  expect(() => validateSourceContract(source, sourceMaterials.slice(0, -1))).toThrow("missing");
  expect(() =>
    validateSourceContract(source, [
      ...sourceMaterials,
      { name: "asset:unbound.bin", bytes: [99] },
    ]),
  ).toThrow("unbound");
  const changedNotice = sourceMaterials.map((material) =>
    material.name === "dependencyNotice:fixture-runtime@1.0.0:NOTICE.txt"
      ? { ...material, bytes: [12] }
      : material,
  );
  expect(() => validateSourceContract(source, changedNotice)).toThrow("noticeDigest");
});

it("rejects accessor elements in nested source arrays without reading them", () => {
  let read = 0;
  const paths = ["src/adapter.ts"];
  Object.defineProperty(paths, 0, {
    enumerable: true,
    get: () => {
      read += 1;
      return "src/adapter.ts";
    },
  });
  expect(() =>
    validateSourceContract({ ...source, source: { ...source.source, paths } }, sourceMaterials),
  ).toThrowError(NativeAdapterError);
  expect(read).toBe(0);
});

it("rejects typed-array own symbols before invoking an iterator", () => {
  let iteratorRead = 0;
  const typedMaterials = sourceMaterials.map((material) => ({
    name: material.name,
    bytes: Uint8Array.from(material.bytes),
  }));
  const typedBytes = typedMaterials[0]?.bytes;
  if (typedBytes === undefined) throw new Error("expected typed material");
  Object.defineProperty(typedBytes, Symbol.iterator, {
    enumerable: false,
    get: () => {
      iteratorRead += 1;
      return () => typedBytes.values();
    },
  });
  expect(() => validateSourceContract(source, typedMaterials)).toThrowError(NativeAdapterError);
  expect(iteratorRead).toBe(0);
});

it("only admits explicit minimal environment names and strips secrets before reading values", () => {
  let secretRead = 0;
  const raw = {
    PATH: "fixtures/bin",
    CI: "1",
    get API_TOKEN() {
      secretRead += 1;
      return "secret";
    },
  } as unknown as Readonly<Record<string, string>>;
  expect(sanitizeEnvironment(raw)).toEqual({ PATH: "fixtures/bin", CI: "1" });
  expect(secretRead).toBe(0);
  const nonSecretAccessor = {} as { readonly PATH?: string };
  Object.defineProperty(nonSecretAccessor, "PATH", {
    enumerable: true,
    get: () => "fixtures/bin",
  });
  expect(() => sanitizeEnvironment(nonSecretAccessor)).toThrowError(NativeAdapterError);
  expect(() => sanitizeEnvironment({ DRIVER_MODE: "fake" })).toThrowError(NativeAdapterError);
  expect(validateLaunchSpec(launch()).environment).toEqual({ PATH: "fixtures/bin" });
});

it("preserves process frame bytes, EOF, close receipt and identity custody", async () => {
  const fake = createFakeProcessPort({ processIdentity: { processId: 123, creationTime: "99" } });
  const session = await openNativeAdapterSession(registry(), {
    driverId: "fixture-driver",
    adapterVersion: "1",
    binding,
    context: context(),
    transport: { kind: "process", port: fake.port, launch: launch() },
  });
  const outgoing = Uint8Array.from([1, 2, 3]);
  await session.send(outgoing, context());
  outgoing[0] = 99;
  const firstWrite = fake.controller.writes.at(0);
  expect(firstWrite).toBeDefined();
  if (firstWrite === undefined) throw new Error("expected fake process write");
  expect([...firstWrite]).toEqual([1, 2, 3]);

  fake.controller.pushFrame(dataFrame([7, 8]));
  const data = await session.receive(context());
  expect(data).toEqual({ kind: "data", bytes: Uint8Array.from([7, 8]) });
  fake.controller.end("fixture EOF");
  await expect(session.receive(context())).resolves.toMatchObject({
    kind: "eof",
    reason: "fixture EOF",
  });
  expect(session.state).toBe("eof");
  await expect(session.send([1], context())).rejects.toMatchObject({ code: "SESSION_STATE_ERROR" });

  const receipt = await session.close("test complete", context());
  expect(receipt).toMatchObject({
    status: "closed",
    processIdentity: { processId: 123, creationTime: "99" },
  });
  await expect(session.close("repeat", context())).resolves.toMatchObject({
    status: "already-closed",
  });
});

it("fails closed on unknown process identity before activation", async () => {
  const fake = createFakeProcessPort({ processIdentity: null });
  await expect(
    openNativeAdapterSession(registry(), {
      driverId: "fixture-driver",
      adapterVersion: "1",
      binding,
      context: context(),
      transport: { kind: "process", port: fake.port, launch: launch() },
    }),
  ).rejects.toMatchObject({ code: "PROCESS_IDENTITY_UNKNOWN" });
  expect(fake.controller.abortReasons).toEqual(["process identity is unknown"]);
  expect(fake.controller.writes).toHaveLength(0);
});

it("aborts exactly once after activation failure and preserves the original error", async () => {
  const original = new NativeAdapterError("PORT_PROTOCOL_ERROR", "activation fixture failed");
  let abortCalls = 0;
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return {
        processIdentity: { processId: 234, creationTime: "1" },
        async activate() {
          throw original;
        },
        async abort() {
          abortCalls += 1;
        },
      };
    },
  };
  await expect(openManagedProcessPort(port, launch(), context())).rejects.toBe(original);
  expect(abortCalls).toBe(1);
});

it("passively snapshots prepared process authority and ignores a later method swap", async () => {
  const original = new NativeAdapterError("PORT_PROTOCOL_ERROR", "activation failed");
  let accessorReads = 0;
  let originalAbortCalls = 0;
  let swappedAbortCalls = 0;
  const prepared = {
    processIdentity: { processId: 238, creationTime: "1" },
    activate: async () => {
      prepared.abort = async () => {
        swappedAbortCalls += 1;
      };
      throw original;
    },
    abort: async () => {
      originalAbortCalls += 1;
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return prepared;
    },
  };
  await expect(openManagedProcessPort(port, launch(), context())).rejects.toBe(original);
  expect(originalAbortCalls).toBe(1);
  expect(swappedAbortCalls).toBe(0);

  const accessorPrepared = {
    processIdentity: { processId: 239, creationTime: "1" },
    activate: async () => ({}) as ActiveBytePort,
    abort: async () => undefined,
  };
  Object.defineProperty(accessorPrepared, "activate", {
    enumerable: true,
    get: () => {
      accessorReads += 1;
      return async () => ({}) as ActiveBytePort;
    },
  });
  await expect(
    openManagedProcessPort(
      { transport: "process", prepare: async () => accessorPrepared },
      launch(),
      context(),
    ),
  ).rejects.toMatchObject({ code: "PORT_PROTOCOL_ERROR" });
  expect(accessorReads).toBe(0);
});

it("captures SDK exchange and close methods once before alias mutation", async () => {
  let exchangeCalls = 0;
  let swappedCalls = 0;
  let closeCalls = 0;
  const transport = {
    transport: "sdk" as const,
    exchange: async (_request: Uint8Array, _context: NativeOperationContext) => {
      exchangeCalls += 1;
      transport.exchange = async () => {
        swappedCalls += 1;
        throw new Error("swapped exchange used");
      };
      return dataFrame([exchangeCalls]);
    },
    close: async () => {
      closeCalls += 1;
    },
  };
  const port = openManagedSdkPort(transport, context());
  await port.write([1], context());
  await expect(port.read(context())).resolves.toEqual({
    kind: "data",
    bytes: Uint8Array.from([1]),
  });
  await port.write([2], context());
  await expect(port.read(context())).resolves.toEqual({
    kind: "data",
    bytes: Uint8Array.from([2]),
  });
  expect(exchangeCalls).toBe(2);
  expect(swappedCalls).toBe(0);
  await port.close("done", context());
  expect(closeCalls).toBe(1);
});

it("aborts a prepared process that resolves after prepare timeout", async () => {
  const gate = deferred<unknown>();
  let abortCalls = 0;
  const prepared = {
    processIdentity: { processId: 240, creationTime: "1" },
    activate: async () => ({}) as ActiveBytePort,
    abort: async () => {
      abortCalls += 1;
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    prepare: async () => gate.promise as Promise<never>,
  };
  await expect(
    openManagedProcessPort(port, launch(), context(binding, Date.now() + 20)),
  ).rejects.toMatchObject({ code: "DEADLINE_EXCEEDED" });
  gate.resolve(prepared);
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
  expect(abortCalls).toBe(1);
});

it("bounds a hanging activation cleanup and retains the cleanup failure as cause", async () => {
  const original = new NativeAdapterError("PORT_PROTOCOL_ERROR", "activation fixture failed");
  let abortCalls = 0;
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return {
        processIdentity: { processId: 235, creationTime: "1" },
        async activate() {
          throw original;
        },
        async abort() {
          abortCalls += 1;
          await new Promise<void>(() => undefined);
        },
      };
    },
  };
  const started = Date.now();
  await expect(openManagedProcessPort(port, launch(), context())).rejects.toMatchObject({
    code: "PORT_PROTOCOL_ERROR",
    cause: { original },
  });
  expect(Date.now() - started).toBeLessThan(2_000);
  expect(abortCalls).toBe(1);
});

async function rejectActiveAdmissionMismatch(
  mismatch: "binding" | "custody" | "identity",
): Promise<void> {
  const activeBinding =
    mismatch === "binding" ? { ...binding, runtimeInstanceId: "wrong-runtime" } : binding;
  const activeCustody = mismatch === "custody" ? "wrong-custody" : "custody-fixture";
  const activeIdentity =
    mismatch === "identity"
      ? { processId: 236, creationTime: "2" }
      : { processId: 236, creationTime: "1" };
  let abortCalls = 0;
  let writeCalls = 0;
  const active: ActiveBytePort = {
    transport: "process",
    binding: activeBinding,
    custodyRef: activeCustody,
    processIdentity: activeIdentity,
    async write() {
      writeCalls += 1;
    },
    async read() {
      return eofFrame("unused");
    },
    async close() {
      return {
        status: "closed" as const,
        reason: "cleanup",
        binding: activeBinding,
        custodyRef: activeCustody,
        processIdentity: activeIdentity,
      };
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return {
        processIdentity: { processId: 236, creationTime: "1" },
        async activate() {
          return active;
        },
        async abort() {
          abortCalls += 1;
        },
      };
    },
  };
  await expect(openManagedProcessPort(port, launch(), context())).rejects.toMatchObject({
    code: "PROCESS_IDENTITY_UNKNOWN",
  });
  expect(abortCalls).toBe(1);
  expect(writeCalls).toBe(0);
}

it("rejects an activated port binding mismatch before I/O", async () => {
  await rejectActiveAdmissionMismatch("binding");
});

it("rejects an activated port custody mismatch before I/O", async () => {
  await rejectActiveAdmissionMismatch("custody");
});

it("rejects an activated port identity mismatch before I/O", async () => {
  await rejectActiveAdmissionMismatch("identity");
});

it("bounds a hanging invalid activated-port close while retaining custody", async () => {
  const wrongBinding = { ...binding, runtimeInstanceId: "wrong-runtime" };
  let abortCalls = 0;
  let closeCalls = 0;
  const active: ActiveBytePort = {
    transport: "process",
    binding: wrongBinding,
    custodyRef: "wrong-custody",
    processIdentity: { processId: 237, creationTime: "2" },
    async write() {},
    async read() {
      return eofFrame("unused");
    },
    async close() {
      closeCalls += 1;
      return await new Promise<never>(() => undefined);
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return {
        processIdentity: { processId: 237, creationTime: "1" },
        async activate() {
          return active;
        },
        async abort() {
          abortCalls += 1;
        },
      };
    },
  };
  const started = Date.now();
  await expect(openManagedProcessPort(port, launch(), context())).rejects.toMatchObject({
    code: "PORT_PROTOCOL_ERROR",
  });
  expect(Date.now() - started).toBeLessThan(1_500);
  expect(abortCalls).toBe(1);
  expect(closeCalls).toBe(1);
});

it("supports multiple runtime instances without a global owner and rejects unknown drivers", async () => {
  const first = createFakeSdkTransport();
  const second = createFakeSdkTransport();
  const firstSession = await openNativeAdapterSession(registry(), {
    driverId: "fixture-driver",
    adapterVersion: "1",
    binding,
    context: context(),
    transport: { kind: "sdk", transport: first.transport },
  });
  const otherBinding = { ...binding, runtimeInstanceId: "runtime-two" };
  const secondSession = await openNativeAdapterSession(registry(), {
    driverId: "fixture-driver",
    adapterVersion: "1",
    binding: otherBinding,
    context: context(otherBinding),
    transport: { kind: "sdk", transport: second.transport },
  });
  expect(firstSession.instanceKey).not.toBe(secondSession.instanceKey);

  await expect(
    openNativeAdapterSession(registry(), {
      driverId: "unknown-driver",
      adapterVersion: "1",
      binding,
      context: context(),
      transport: { kind: "sdk", transport: first.transport },
    }),
  ).rejects.toMatchObject({ code: "UNKNOWN_DRIVER" });
  await expect(
    openNativeAdapterSession(registry(), {
      driverId: "fixture-driver",
      adapterVersion: "2",
      binding,
      context: context(),
      transport: { kind: "sdk", transport: first.transport },
    }),
  ).rejects.toMatchObject({ code: "DRIVER_VERSION_MISMATCH" });
  await firstSession.close("done", context());
  await secondSession.close("done", context(otherBinding));
});

it("snapshots registration before reading fields and rejects accessors, symbols and extras", () => {
  let read = 0;
  const registration = {
    driverId: "fixture-driver",
    adapterVersion: "1",
    source,
    sourceMaterials,
    transports: ["sdk" as const],
  };
  Object.defineProperty(registration, "driverId", {
    enumerable: true,
    get: () => {
      read += 1;
      return "fixture-driver";
    },
  });
  expect(() => new NativeAdapterRegistry([registration])).toThrowError(NativeAdapterError);
  expect(read).toBe(0);
  expect(
    () =>
      new NativeAdapterRegistry([
        { ...registration, driverId: "fixture-driver", extra: true } as never,
      ]),
  ).toThrowError(NativeAdapterError);
});

it("snapshots the outer registration array before iteration", () => {
  let read = 0;
  const registration = {
    driverId: "fixture-driver",
    adapterVersion: "1",
    source,
    sourceMaterials,
    transports: ["sdk" as const],
  };
  const registrations = [registration];
  Object.defineProperty(registrations, 0, {
    enumerable: true,
    get: () => {
      read += 1;
      return registration;
    },
  });
  expect(() => new NativeAdapterRegistry(registrations)).toThrowError(NativeAdapterError);
  expect(read).toBe(0);
});

async function rejectCloseReceiptMismatch(
  mismatch: "binding" | "custody" | "identity",
): Promise<void> {
  const receiptBinding =
    mismatch === "binding" ? { ...binding, runtimeInstanceId: "other-runtime" } : binding;
  const receiptCustody = mismatch === "custody" ? "wrong-custody" : "custody-fixture";
  const receiptIdentity =
    mismatch === "identity"
      ? { processId: 346, creationTime: "1" }
      : { processId: 345, creationTime: "1" };
  const active: ActiveBytePort = {
    transport: "process",
    binding,
    custodyRef: "custody-fixture",
    processIdentity: { processId: 345, creationTime: "1" },
    async write() {},
    async read() {
      return eofFrame("not used");
    },
    async close(reason) {
      return {
        status: "closed" as const,
        reason,
        binding: receiptBinding,
        custodyRef: receiptCustody,
        processIdentity: receiptIdentity,
      };
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare() {
      return {
        processIdentity: { processId: 345, creationTime: "1" },
        async activate() {
          return active;
        },
        async abort() {},
      };
    },
  };
  const session = await openNativeAdapterSession(registry(), {
    driverId: "fixture-driver",
    adapterVersion: "1",
    binding,
    context: context(),
    transport: { kind: "process", port, launch: launch() },
  });
  await expect(session.close("mismatch", context())).rejects.toMatchObject({
    code: "PROCESS_IDENTITY_UNKNOWN",
  });
  expect(session.state).toBe("failed");
}

it("rejects a close receipt binding mismatch independently", async () => {
  await rejectCloseReceiptMismatch("binding");
});

it("rejects a close receipt custody mismatch independently", async () => {
  await rejectCloseReceiptMismatch("custody");
});

it("rejects a close receipt identity mismatch independently", async () => {
  await rejectCloseReceiptMismatch("identity");
});

it("keeps protocol errors and cancellation observable", async () => {
  const fake = createFakeSdkTransport();
  const session = await openNativeAdapterSession(registry(), {
    driverId: "fixture-driver",
    adapterVersion: "1",
    binding,
    context: context(),
    transport: { kind: "sdk", transport: fake.transport },
  });
  await session.send([4], context());
  fake.controller.fail("NATIVE_BAD_FRAME", "fixture rejected frame", false);
  await expect(session.receive(context())).resolves.toEqual(
    errorFrame("NATIVE_BAD_FRAME", "fixture rejected frame", false),
  );

  await session.send([5], context());
  const abort = new AbortController();
  const receive = session.receive(context(binding, Date.now() + 5_000, abort.signal));
  await Promise.resolve();
  abort.abort("caller stopped");
  await expect(receive).rejects.toMatchObject({
    code: "CANCELLED",
  });
  expect(session.state).toBe("cancelled");
  await session.close("cancelled cleanup", context());
});

it("requires exact frame variants and a boolean fatal flag", () => {
  expect(() =>
    cloneFrame({ kind: "error", code: "E", message: "bad", fatal: 1 as never }),
  ).toThrowError(NativeAdapterError);
  expect(() => cloneFrame({ kind: "eof", reason: "done", extra: true } as never)).toThrowError(
    NativeAdapterError,
  );
});
