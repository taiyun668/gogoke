import {
  assertOperationUsable,
  awaitOperation,
  cloneBytes,
  cloneFrame,
  NativeAdapterError,
  type NativeBytes,
  type NativeCloseReceipt,
  type NativeFrame,
  type NativeLaunchSpec,
  type NativeAdapterBinding,
  type ProcessIdentity,
  validateProcessIdentity,
  validateLaunchSpec,
} from "./types.ts";
import type {
  ActiveBytePort,
  ManagedProcessPort,
  ManagedSdkTransport,
  PreparedProcessPort,
} from "./ports.ts";

interface Deferred<T> {
  readonly promise: Promise<T>;
  readonly resolve: (value: T) => void;
  readonly reject: (reason: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolveValue, rejectValue) => {
    resolve = resolveValue;
    reject = rejectValue;
  });
  return { promise, resolve, reject };
}

interface FakeProcessState {
  readonly processIdentity: ProcessIdentity | null;
  binding: Readonly<NativeAdapterBinding> | undefined;
  custodyRef: string | undefined;
  readonly frameLimit: number;
  readonly launches: NativeLaunchSpec[];
  readonly writes: NativeBytes[];
  readonly abortReasons: string[];
  readonly closeReasons: string[];
  readonly incoming: NativeFrame[];
  readonly pendingReads: Deferred<Readonly<NativeFrame>>[];
  active: boolean;
  prepared: boolean;
  aborted: boolean;
  closed: boolean;
  terminalFrame: Readonly<NativeFrame> | undefined;
  closeReceipt: NativeCloseReceipt | undefined;
}

export interface FakeProcessController {
  readonly launches: ReadonlyArray<Readonly<NativeLaunchSpec>>;
  readonly writes: ReadonlyArray<NativeBytes>;
  readonly abortReasons: ReadonlyArray<string>;
  readonly closeReasons: ReadonlyArray<string>;
  pushFrame(frame: NativeFrame): void;
  end(reason?: string): void;
  fail(code: string, message: string, fatal?: boolean): void;
}

export interface FakeProcessOptions {
  readonly processIdentity?: ProcessIdentity | null;
  readonly maxFrameBytes?: number;
}

let nextFakePid = 40_000;

function defaultIdentity(): ProcessIdentity {
  const processId = nextFakePid++;
  return Object.freeze({ processId, creationTime: String(processId * 10_000) });
}

function portClosed(): NativeAdapterError {
  return new NativeAdapterError("PORT_CLOSED", "fake byte port is closed");
}

function activePort(state: FakeProcessState): ActiveBytePort {
  return {
    transport: "process",
    binding: state.binding!,
    custodyRef: state.custodyRef!,
    processIdentity: state.processIdentity,
    async write(bytes, context) {
      assertOperationUsable(context);
      if (!state.active || state.closed || state.aborted) throw portClosed();
      const copied = cloneBytes(bytes, "fake.write", state.frameLimit);
      state.writes.push(copied);
    },
    async read(context) {
      assertOperationUsable(context);
      if (state.closed || state.aborted) throw portClosed();
      if (state.terminalFrame !== undefined)
        return cloneFrame(state.terminalFrame, state.frameLimit);
      const queued = state.incoming.shift();
      if (queued !== undefined) {
        if (queued.kind === "eof" || (queued.kind === "error" && queued.fatal)) {
          state.terminalFrame = queued;
        }
        return cloneFrame(queued, state.frameLimit);
      }
      if (state.pendingReads.length > 0) {
        throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "fake process allows one pending read");
      }
      const pending = deferred<Readonly<NativeFrame>>();
      state.pendingReads.push(pending);
      try {
        return await awaitOperation(pending.promise, context);
      } catch (error) {
        throw error instanceof NativeAdapterError
          ? error
          : new NativeAdapterError("PORT_PROTOCOL_ERROR", "fake read failed", error);
      }
    },
    async close(reason, context) {
      assertOperationUsable(context);
      if (state.closeReceipt !== undefined) {
        return Object.freeze({ ...state.closeReceipt, status: "already-closed" as const });
      }
      state.closed = true;
      state.active = false;
      state.closeReasons.push(reason);
      const error = portClosed();
      for (const pending of state.pendingReads.splice(0)) pending.reject(error);
      state.closeReceipt = Object.freeze({
        status: "closed" as const,
        reason,
        binding: state.binding!,
        custodyRef: state.custodyRef!,
        processIdentity: state.processIdentity,
      });
      return state.closeReceipt;
    },
  };
}

export function createFakeProcessPort(options: FakeProcessOptions = {}): {
  readonly port: ManagedProcessPort;
  readonly controller: FakeProcessController;
} {
  const identity =
    options.processIdentity === undefined
      ? defaultIdentity()
      : options.processIdentity === null
        ? null
        : validateProcessIdentity(options.processIdentity);
  const frameLimit = options.maxFrameBytes ?? 4 * 1024 * 1024;
  if (!Number.isSafeInteger(frameLimit) || frameLimit <= 0) {
    throw new NativeAdapterError("INVALID_FRAME", "fake maxFrameBytes must be positive");
  }
  const state: FakeProcessState = {
    processIdentity: identity,
    binding: undefined,
    custodyRef: undefined,
    frameLimit,
    launches: [],
    writes: [],
    abortReasons: [],
    closeReasons: [],
    incoming: [],
    pendingReads: [],
    active: false,
    prepared: false,
    aborted: false,
    closed: false,
    terminalFrame: undefined,
    closeReceipt: undefined,
  };
  const controller: FakeProcessController = {
    get launches() {
      return Object.freeze([...state.launches]);
    },
    get writes() {
      return Object.freeze(state.writes.map((bytes) => new Uint8Array(bytes)));
    },
    get abortReasons() {
      return Object.freeze([...state.abortReasons]);
    },
    get closeReasons() {
      return Object.freeze([...state.closeReasons]);
    },
    pushFrame(frame) {
      const copied = cloneFrame(frame, frameLimit);
      const pending = state.pendingReads.shift();
      if (pending !== undefined) {
        if (copied.kind === "eof" || (copied.kind === "error" && copied.fatal)) {
          state.terminalFrame = copied;
        }
        pending.resolve(copied);
      } else {
        state.incoming.push(copied);
      }
    },
    end(reason = "fake process EOF") {
      controller.pushFrame({ kind: "eof", reason });
    },
    fail(code, message, fatal = true) {
      controller.pushFrame({ kind: "error", code, message, fatal });
    },
  };
  const port: ManagedProcessPort = {
    transport: "process",
    async prepare(spec, context): Promise<PreparedProcessPort> {
      assertOperationUsable(context);
      if (state.prepared || state.closed || state.aborted) {
        throw new NativeAdapterError(
          "PORT_PROTOCOL_ERROR",
          "fake process was prepared more than once",
        );
      }
      const launch = validateLaunchSpec(spec);
      state.binding = launch.binding;
      state.custodyRef = launch.custodyRef;
      state.prepared = true;
      state.launches.push(launch);
      return {
        processIdentity: identity,
        async activate(activateContext) {
          assertOperationUsable(activateContext);
          if (!state.prepared || state.aborted || state.closed) {
            throw new NativeAdapterError("PORT_NOT_READY", "fake process is not activatable");
          }
          if (state.active)
            throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "fake process is already active");
          state.active = true;
          return activePort(state);
        },
        async abort(reason, abortContext) {
          assertOperationUsable(abortContext);
          state.aborted = true;
          state.abortReasons.push(reason);
          state.active = false;
        },
      };
    },
  };
  return { port, controller };
}

interface FakeSdkState {
  readonly requests: NativeBytes[];
  readonly responses: NativeFrame[];
  readonly pending: Deferred<NativeFrame>[];
  readonly closeReasons: string[];
  closed: boolean;
}

export interface FakeSdkController {
  readonly requests: ReadonlyArray<NativeBytes>;
  readonly closeReasons: ReadonlyArray<string>;
  pushResponse(frame: NativeFrame): void;
  end(reason?: string): void;
  fail(code: string, message: string, fatal?: boolean): void;
}

/** An in-memory SDK transport; it never opens a client or consults credentials. */
export function createFakeSdkTransport(options: { readonly maxFrameBytes?: number } = {}): {
  readonly transport: ManagedSdkTransport;
  readonly controller: FakeSdkController;
} {
  const frameLimit = options.maxFrameBytes ?? 4 * 1024 * 1024;
  if (!Number.isSafeInteger(frameLimit) || frameLimit <= 0) {
    throw new NativeAdapterError("INVALID_FRAME", "fake maxFrameBytes must be positive");
  }
  const state: FakeSdkState = {
    requests: [],
    responses: [],
    pending: [],
    closeReasons: [],
    closed: false,
  };
  const controller: FakeSdkController = {
    get requests() {
      return Object.freeze(state.requests.map((bytes) => new Uint8Array(bytes)));
    },
    get closeReasons() {
      return Object.freeze([...state.closeReasons]);
    },
    pushResponse(frame) {
      const copied = cloneFrame(frame, frameLimit);
      const pending = state.pending.shift();
      if (pending !== undefined) pending.resolve(copied);
      else state.responses.push(copied);
    },
    end(reason = "fake SDK EOF") {
      controller.pushResponse({ kind: "eof", reason });
    },
    fail(code, message, fatal = true) {
      controller.pushResponse({ kind: "error", code, message, fatal });
    },
  };
  const transport: ManagedSdkTransport = {
    transport: "sdk",
    async exchange(request, context) {
      assertOperationUsable(context);
      if (state.closed) throw portClosed();
      state.requests.push(cloneBytes(request, "fake.sdk.request", frameLimit));
      const response = state.responses.shift();
      if (response !== undefined) return cloneFrame(response, frameLimit);
      const pending = deferred<NativeFrame>();
      state.pending.push(pending);
      return await pending.promise;
    },
    async close(reason, context) {
      assertOperationUsable(context);
      if (state.closed) return;
      state.closed = true;
      state.closeReasons.push(reason);
      for (const pending of state.pending.splice(0)) pending.reject(portClosed());
    },
  };
  return { transport, controller };
}
