import {
  assertBindingMatches,
  assertOperationUsable,
  awaitOperation,
  cloneBytes,
  cloneFrame,
  NativeAdapterError,
  type ByteInput,
  type NativeBytes,
  type NativeCloseReceipt,
  type NativeFrame,
  type NativeLaunchSpec,
  type NativeOperationContext,
  type ProcessIdentity,
  type NativeAdapterBinding,
  validateLaunchSpec,
  validateBinding,
  validateProcessIdentity,
  snapshotPassiveRecord,
  bindingKey,
} from "./types.ts";

/**
 * A port is the only side-effect boundary visible to the adapter session.
 * Production wiring is intentionally not supplied here: the port may be
 * backed by a native host, while tests use the in-memory implementation in
 * fake.ts. There is no process spawn, account, network or product-store API.
 */
export interface ActiveBytePort {
  readonly transport: "process" | "sdk";
  readonly binding: Readonly<NativeAdapterBinding>;
  readonly custodyRef: string;
  readonly processIdentity: ProcessIdentity | null;
  write(bytes: ByteInput, context: NativeOperationContext): Promise<void>;
  read(context: NativeOperationContext): Promise<Readonly<NativeFrame>>;
  close(reason: string, context: NativeOperationContext): Promise<NativeCloseReceipt>;
}

export interface PreparedProcessPort {
  readonly processIdentity: ProcessIdentity | null;
  activate(context: NativeOperationContext): Promise<ActiveBytePort>;
  abort(reason: string, context: NativeOperationContext): Promise<void>;
}

export interface ManagedProcessPort {
  readonly transport: "process";
  prepare(spec: NativeLaunchSpec, context: NativeOperationContext): Promise<PreparedProcessPort>;
}

/**
 * SDK boundary: the wrapper exchanges opaque bytes and protocol frames only.
 * It does not serialize objects, inspect credentials, or acquire a network
 * client. The transport remains externally owned by its caller.
 */
export interface ManagedSdkTransport {
  readonly transport: "sdk";
  exchange(request: NativeBytes, context: NativeOperationContext): Promise<NativeFrame>;
  close(reason: string, context: NativeOperationContext): Promise<void>;
}

function maxFrameSize(value: number): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new NativeAdapterError("INVALID_FRAME", "maxFrameBytes must be a positive safe integer");
  }
  return value;
}

function normalizePortFailure(error: unknown, operation: string): NativeAdapterError {
  if (error instanceof NativeAdapterError) return error;
  return new NativeAdapterError("PORT_PROTOCOL_ERROR", `${operation} failed`, error);
}

function sameIdentity(left: ProcessIdentity, right: ProcessIdentity): boolean {
  return left.processId === right.processId && left.creationTime === right.creationTime;
}

type ActiveMethod = (...args: ReadonlyArray<unknown>) => Promise<unknown>;

function captureMethod(
  raw: object,
  record: Readonly<Record<string, unknown>>,
  name: string,
): ActiveMethod {
  const own = record[name];
  if (own !== undefined) {
    if (typeof own !== "function") {
      throw new NativeAdapterError("PORT_PROTOCOL_ERROR", `activePort.${name} must be a function`);
    }
    return own as ActiveMethod;
  }
  let prototype: object | null = Object.getPrototypeOf(raw);
  while (prototype !== null) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    if (descriptor !== undefined) {
      if (!("value" in descriptor) || typeof descriptor.value !== "function") {
        throw new NativeAdapterError(
          "PORT_PROTOCOL_ERROR",
          `activePort.${name} must be a data method`,
        );
      }
      return descriptor.value as ActiveMethod;
    }
    prototype = Object.getPrototypeOf(prototype);
  }
  throw new NativeAdapterError("PORT_PROTOCOL_ERROR", `activePort.${name} is missing`);
}

function snapshotActiveProcessPort(
  value: unknown,
  expectedBinding: Readonly<NativeAdapterBinding>,
  expectedCustodyRef: string,
  expectedIdentity: Readonly<ProcessIdentity>,
): ActiveBytePort {
  const record = snapshotPassiveRecord(value, "activePort", undefined, "PORT_PROTOCOL_ERROR");
  if (record.transport !== "process") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "activated port transport is invalid");
  }
  const binding = validateBinding(record.binding as NativeAdapterBinding);
  if (bindingKey(binding) !== bindingKey(expectedBinding)) {
    throw new NativeAdapterError("PROCESS_IDENTITY_UNKNOWN", "activated port binding changed");
  }
  if (record.custodyRef !== expectedCustodyRef) {
    throw new NativeAdapterError("PROCESS_IDENTITY_UNKNOWN", "activated port custody changed");
  }
  if (record.processIdentity === null || record.processIdentity === undefined) {
    throw new NativeAdapterError("PROCESS_IDENTITY_UNKNOWN", "activated port identity is unknown");
  }
  const processIdentity = validateProcessIdentity(record.processIdentity as ProcessIdentity);
  if (!sameIdentity(processIdentity, expectedIdentity)) {
    throw new NativeAdapterError("PROCESS_IDENTITY_UNKNOWN", "activated port identity changed");
  }
  const raw = value as object;
  const write = captureMethod(raw, record, "write");
  const read = captureMethod(raw, record, "read");
  const close = captureMethod(raw, record, "close");
  return {
    transport: "process",
    binding,
    custodyRef: expectedCustodyRef,
    processIdentity,
    write(bytes, context) {
      return Promise.resolve().then(() => write.call(raw, bytes, context)) as Promise<void>;
    },
    read(context) {
      return Promise.resolve().then(
        () => read.call(raw, context),
      ) as Promise<Readonly<NativeFrame>>;
    },
    close(reason, context) {
      return Promise.resolve().then(
        () => close.call(raw, reason, context),
      ) as Promise<NativeCloseReceipt>;
    },
  };
}

function snapshotPreparedProcessPort(value: unknown): PreparedProcessPort {
  const record = snapshotPassiveRecord(
    value,
    "preparedPort",
    ["processIdentity", "activate", "abort"],
    "PORT_PROTOCOL_ERROR",
  );
  if (typeof record.activate !== "function") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "preparedPort.activate must be a function");
  }
  if (typeof record.abort !== "function") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "preparedPort.abort must be a function");
  }
  const processIdentity =
    record.processIdentity === null
      ? null
      : validateProcessIdentity(record.processIdentity as ProcessIdentity);
  const raw = value as object;
  const activate = (record.activate as (...args: ReadonlyArray<unknown>) => unknown).bind(raw);
  const abort = (record.abort as (...args: ReadonlyArray<unknown>) => unknown).bind(raw);
  return Object.freeze({
    processIdentity,
    activate(context: NativeOperationContext) {
      return activate(context) as Promise<ActiveBytePort>;
    },
    abort(reason: string, context: NativeOperationContext) {
      return abort(reason, context) as Promise<void>;
    },
  });
}

function snapshotManagedSdkTransport(value: unknown): ManagedSdkTransport {
  const record = snapshotPassiveRecord(
    value,
    "sdkTransport",
    ["transport", "exchange", "close"],
    "PORT_PROTOCOL_ERROR",
  );
  if (record.transport !== "sdk") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "SDK transport has an invalid kind");
  }
  if (typeof record.exchange !== "function") {
    throw new NativeAdapterError(
      "PORT_PROTOCOL_ERROR",
      "SDK transport exchange must be a function",
    );
  }
  if (typeof record.close !== "function") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "SDK transport close must be a function");
  }
  const raw = value as object;
  const exchange = (record.exchange as (...args: ReadonlyArray<unknown>) => unknown).bind(raw);
  const close = (record.close as (...args: ReadonlyArray<unknown>) => unknown).bind(raw);
  return Object.freeze({
    transport: "sdk" as const,
    exchange(request: NativeBytes, context: NativeOperationContext) {
      return exchange(request, context) as Promise<NativeFrame>;
    },
    close(reason: string, context: NativeOperationContext) {
      return close(reason, context) as Promise<void>;
    },
  });
}

function cleanupContext(context: NativeOperationContext): NativeOperationContext {
  return {
    signal: new AbortController().signal,
    deadlineAt: Date.now() + 1_000,
    binding: context.binding,
    custodyRef: context.custodyRef,
  };
}

async function boundedCloseInvalidActivePort(
  value: unknown,
  reason: string,
  context: NativeOperationContext,
): Promise<"completed" | "failed"> {
  try {
    const record = snapshotPassiveRecord(
      value,
      "invalidActivePort",
      undefined,
      "PORT_PROTOCOL_ERROR",
    );
    const raw = value as object;
    const close = captureMethod(raw, record, "close");
    const cleanup = cleanupContext(context);
    await awaitOperation(
      Promise.resolve().then(() => close.call(raw, reason, cleanup)),
      cleanup,
    );
    return "completed";
  } catch {
    return "failed";
  }
}

function assertPortContext(
  binding: Readonly<NativeAdapterBinding>,
  custodyRef: string,
  context: NativeOperationContext,
): void {
  assertOperationUsable(context);
  assertBindingMatches(binding, context.binding);
  if (context.custodyRef !== custodyRef) {
    throw new NativeAdapterError(
      "INVALID_LAUNCH",
      "operation custody reference does not match the admitted port",
    );
  }
}

function guardActivePort(
  active: ActiveBytePort,
  binding: Readonly<NativeAdapterBinding>,
  custodyRef: string,
  admittedIdentity: ProcessIdentity | null = active.processIdentity,
): ActiveBytePort {
  let closed = false;
  let poisoned: NativeAdapterError | undefined;
  let closePromise: Promise<NativeCloseReceipt> | undefined;
  return {
    transport: active.transport,
    binding,
    custodyRef,
    processIdentity: admittedIdentity,
    async write(bytes, context) {
      assertPortContext(binding, custodyRef, context);
      if (closed) throw new NativeAdapterError("PORT_CLOSED", "byte port is closed");
      if (poisoned !== undefined) throw poisoned;
      try {
        await awaitOperation(
          Promise.resolve().then(() => active.write(bytes, context)),
          context,
        );
      } catch (error) {
        const normalized = normalizePortFailure(error, "port write");
        if (normalized.code === "CANCELLED" || normalized.code === "DEADLINE_EXCEEDED")
          poisoned = normalized;
        throw normalized;
      }
    },
    async read(context) {
      assertPortContext(binding, custodyRef, context);
      if (closed) throw new NativeAdapterError("PORT_CLOSED", "byte port is closed");
      if (poisoned !== undefined) throw poisoned;
      try {
        return await awaitOperation(
          Promise.resolve().then(() => active.read(context)),
          context,
        );
      } catch (error) {
        const normalized = normalizePortFailure(error, "port read");
        if (normalized.code === "CANCELLED" || normalized.code === "DEADLINE_EXCEEDED")
          poisoned = normalized;
        throw normalized;
      }
    },
    async close(reason, context) {
      assertPortContext(binding, custodyRef, context);
      if (closePromise !== undefined) return closePromise;
      closed = true;
      closePromise = awaitOperation(
        Promise.resolve()
          .then(() => active.close(reason, context))
          .catch((error: unknown) => {
            throw normalizePortFailure(error, "port close");
          }),
        context,
      );
      return closePromise;
    },
  };
}

/**
 * Adapt an externally-owned SDK byte transport to the same active-port
 * contract as a managed process. One write is paired with one read; request
 * and response bytes are copied at both sides of the boundary.
 */
export function openManagedSdkPort(
  transport: ManagedSdkTransport,
  context: NativeOperationContext,
  maxFrameBytes = 4 * 1024 * 1024,
): ActiveBytePort {
  assertOperationUsable(context);
  const frameLimit = maxFrameSize(maxFrameBytes);
  const capturedTransport = snapshotManagedSdkTransport(transport);
  let pending: NativeBytes | undefined;
  let closed = false;
  let poisoned: NativeAdapterError | undefined;
  let exchange: Promise<Readonly<NativeFrame>> | undefined;
  let closePromise: Promise<NativeCloseReceipt> | undefined;
  let closeReceipt: NativeCloseReceipt | undefined;
  return {
    transport: "sdk",
    binding: validateBinding(context.binding),
    custodyRef: context.custodyRef,
    processIdentity: null,
    async write(bytes, operation) {
      assertPortContext(validateBinding(context.binding), context.custodyRef, operation);
      if (closed) throw new NativeAdapterError("PORT_CLOSED", "SDK port is closed");
      if (poisoned !== undefined) throw poisoned;
      if (pending !== undefined || exchange !== undefined) {
        throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "SDK port has an unread request");
      }
      await awaitOperation(
        Promise.resolve().then(() => {
          pending = cloneBytes(bytes, "sdk.request", frameLimit);
        }),
        operation,
      );
    },
    async read(operation) {
      assertPortContext(validateBinding(context.binding), context.custodyRef, operation);
      if (closed) throw new NativeAdapterError("PORT_CLOSED", "SDK port is closed");
      if (poisoned !== undefined) throw poisoned;
      if (pending === undefined && exchange === undefined) {
        throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "SDK read has no pending request");
      }
      if (exchange === undefined) {
        const request = pending;
        pending = undefined;
        exchange = Promise.resolve()
          .then(() =>
            capturedTransport.exchange(cloneBytes(request!, "sdk.request", frameLimit), operation),
          )
          .then((response) => cloneFrame(response, frameLimit))
          .catch((error: unknown) => {
            throw normalizePortFailure(error, "SDK exchange");
          });
        const current = exchange;
        void current.then(
          () => {
            if (exchange === current) exchange = undefined;
          },
          () => {
            if (exchange === current) exchange = undefined;
          },
        );
      }
      const current = exchange;
      if (current === undefined) {
        throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "SDK exchange was not created");
      }
      try {
        const response = await awaitOperation(current, operation);
        if (exchange === current) exchange = undefined;
        return response;
      } catch (error) {
        const normalized = normalizePortFailure(error, "SDK read");
        if (
          normalized.code !== "CANCELLED" &&
          normalized.code !== "DEADLINE_EXCEEDED" &&
          exchange === current
        ) {
          exchange = undefined;
        }
        if (normalized.code === "CANCELLED" || normalized.code === "DEADLINE_EXCEEDED") {
          poisoned = normalized;
        }
        throw normalized;
      }
    },
    async close(reason, operation) {
      assertPortContext(validateBinding(context.binding), context.custodyRef, operation);
      if (closeReceipt !== undefined) {
        return Object.freeze({ ...closeReceipt, status: "already-closed" as const });
      }
      if (closePromise !== undefined) return closePromise;
      closed = true;
      pending = undefined;
      closePromise = awaitOperation(
        Promise.resolve()
          .then(() => capturedTransport.close(reason, operation))
          .catch((error: unknown) => {
            throw normalizePortFailure(error, "SDK close");
          })
          .then(() => {
            closeReceipt = Object.freeze({
              status: "closed" as const,
              reason,
              binding: validateBinding(context.binding),
              custodyRef: context.custodyRef,
              processIdentity: null,
            });
            return closeReceipt;
          }),
        operation,
      );
      return closePromise;
    },
  };
}

/**
 * Prepare and activate through an externally-owned process port. Unknown or
 * changing identities are hard failures; the adapter never turns a missing
 * identity into a writable session.
 */
export async function openManagedProcessPort(
  port: ManagedProcessPort,
  spec: NativeLaunchSpec,
  context: NativeOperationContext,
): Promise<ActiveBytePort> {
  assertOperationUsable(context);
  if (port.transport !== "process") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "process port has an invalid kind");
  }
  const launch = validateLaunchSpec(spec);
  assertBindingMatches(launch.binding, context.binding);
  if (launch.custodyRef !== context.custodyRef) {
    throw new NativeAdapterError(
      "INVALID_LAUNCH",
      "launch custody reference does not match operation custody",
    );
  }
  let abortInFlight: Promise<"completed" | "failed"> | undefined;
  const abortPreparedOnce = (
    prepared: PreparedProcessPort | undefined,
    reason: string,
  ): Promise<"completed" | "failed"> => {
    if (abortInFlight !== undefined) return abortInFlight;
    if (prepared === undefined) return Promise.resolve("failed");
    abortInFlight = (async () => {
      const cleanupContext: NativeOperationContext = {
        signal: new AbortController().signal,
        deadlineAt: Date.now() + 1_000,
        binding: context.binding,
        custodyRef: context.custodyRef,
      };
      try {
        await awaitOperation(
          Promise.resolve().then(() => prepared.abort(reason, cleanupContext)),
          cleanupContext,
        );
        return "completed";
      } catch {
        return "failed";
      }
    })();
    return abortInFlight;
  };
  let prepareTimedOut = false;
  const prepareOperation = Promise.resolve().then(() => port.prepare(launch, context));
  void prepareOperation.then(
    (rawPrepared) => {
      if (!prepareTimedOut) return;
      try {
        const latePrepared = snapshotPreparedProcessPort(rawPrepared);
        void abortPreparedOnce(latePrepared, "prepare completed after deadline");
      } catch {
        // A late malformed result cannot be safely invoked; its rejection is consumed.
      }
    },
    () => undefined,
  );
  let rawPrepared: unknown;
  let prepared: PreparedProcessPort;
  try {
    rawPrepared = await awaitOperation(prepareOperation, context);
    prepared = snapshotPreparedProcessPort(rawPrepared);
  } catch (error) {
    const timedOut =
      error instanceof NativeAdapterError &&
      (error.code === "CANCELLED" || error.code === "DEADLINE_EXCEEDED");
    if (timedOut) prepareTimedOut = true;
    if (!timedOut && rawPrepared !== undefined) {
      try {
        const invalidPrepared = snapshotPreparedProcessPort(rawPrepared);
        await abortPreparedOnce(invalidPrepared, "prepared port validation failed");
      } catch {
        // Preserve the original validation error and do not invoke untrusted members.
      }
    }
    throw error;
  }
  let preparedIdentity: ProcessIdentity | null;
  try {
    preparedIdentity =
      prepared.processIdentity === null ? null : validateProcessIdentity(prepared.processIdentity);
  } catch (error) {
    await abortPreparedOnce(prepared, "prepared process identity is invalid");
    throw error;
  }
  if (preparedIdentity === null) {
    await abortPreparedOnce(prepared, "process identity is unknown");
    throw new NativeAdapterError(
      "PROCESS_IDENTITY_UNKNOWN",
      "managed process cannot become writable without a process identity",
    );
  }
  let activeRaw: unknown;
  try {
    activeRaw = await awaitOperation(
      Promise.resolve().then(() => prepared.activate(context)),
      context,
    );
  } catch (error) {
    const cleanup = await abortPreparedOnce(prepared, "activation failed");
    if (cleanup === "failed") {
      throw new NativeAdapterError(
        "PORT_PROTOCOL_ERROR",
        "process activation failed and abort cleanup failed",
        {
          original: error,
          cleanup,
        },
      );
    }
    throw error;
  }
  let admittedActive: ActiveBytePort;
  try {
    admittedActive = snapshotActiveProcessPort(
      activeRaw,
      validateBinding(context.binding),
      context.custodyRef,
      preparedIdentity,
    );
  } catch (error) {
    const [closeResult, abortResult] = await Promise.all([
      boundedCloseInvalidActivePort(activeRaw, "activated port admission failed", context),
      abortPreparedOnce(prepared, "activated port admission failed"),
    ]);
    if (closeResult === "failed" || abortResult === "failed") {
      throw new NativeAdapterError(
        "PORT_PROTOCOL_ERROR",
        "activated port admission failed and cleanup failed",
        {
          original: error,
          closeResult,
          abortResult,
        },
      );
    }
    throw error;
  }
  return guardActivePort(
    admittedActive,
    validateBinding(context.binding),
    context.custodyRef,
    admittedActive.processIdentity!,
  );
}
