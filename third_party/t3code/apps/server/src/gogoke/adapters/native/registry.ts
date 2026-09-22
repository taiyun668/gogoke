import {
  assertBindingMatches,
  assertOperationUsable,
  bindingKey,
  cloneBytes,
  cloneFrame,
  NativeAdapterError,
  type ByteInput,
  type NativeAdapterBinding,
  type NativeAdapterSourceContract,
  type NativeAdapterSourceContractInput,
  type SourceMaterialInput,
  type NativeCloseReceipt,
  type NativeFrame,
  type NativeLaunchSpec,
  type NativeOperationContext,
  type ProcessIdentity,
  validateBinding,
  validateSourceContract,
  snapshotPassiveArray,
  snapshotPassiveRecord,
  validateProcessIdentity,
} from "./types.ts";
import {
  openManagedProcessPort,
  openManagedSdkPort,
  type ActiveBytePort,
  type ManagedProcessPort,
  type ManagedSdkTransport,
} from "./ports.ts";

export type NativeTransportKind = "process" | "sdk";

export interface NativeAdapterDescriptor {
  readonly driverId: string;
  readonly adapterVersion: string;
  readonly source: NativeAdapterSourceContract;
  readonly transports: ReadonlyArray<NativeTransportKind>;
}

export interface NativeAdapterRegistration {
  readonly driverId: string;
  readonly adapterVersion: string;
  readonly source: NativeAdapterSourceContractInput;
  readonly sourceMaterials: ReadonlyArray<SourceMaterialInput>;
  readonly transports: ReadonlyArray<NativeTransportKind>;
}

export type NativeTransportRequest =
  | {
      readonly kind: "process";
      readonly port: ManagedProcessPort;
      readonly launch: NativeLaunchSpec;
    }
  | {
      readonly kind: "sdk";
      readonly transport: ManagedSdkTransport;
    };

export interface OpenNativeAdapterRequest {
  readonly driverId: string;
  readonly adapterVersion: string;
  readonly binding: NativeAdapterBinding;
  readonly context: NativeOperationContext;
  readonly transport: NativeTransportRequest;
  readonly maxFrameBytes?: number;
}

export type NativeSessionState = "active" | "eof" | "failed" | "cancelled" | "closing" | "closed";

export interface NativeAdapterSession {
  readonly driverId: string;
  readonly adapterVersion: string;
  readonly source: Readonly<NativeAdapterSourceContract>;
  readonly binding: Readonly<NativeAdapterBinding>;
  readonly custodyRef: string;
  readonly instanceKey: string;
  readonly processIdentity: ProcessIdentity | null;
  readonly state: NativeSessionState;
  send(bytes: ByteInput, context: NativeOperationContext): Promise<void>;
  receive(context: NativeOperationContext): Promise<Readonly<NativeFrame>>;
  close(reason: string, context: NativeOperationContext): Promise<NativeCloseReceipt>;
}

function key(driverId: string, adapterVersion: string): string {
  return `${driverId}\u0000${adapterVersion}`;
}

function canonicalDescriptor(raw: NativeAdapterRegistration): Readonly<NativeAdapterDescriptor> {
  if (typeof raw !== "object" || raw === null) {
    throw new NativeAdapterError("INVALID_DESCRIPTOR", "adapter registration must be an object");
  }
  const registration = snapshotPassiveRecord(
    raw,
    "registration",
    ["driverId", "adapterVersion", "source", "sourceMaterials", "transports"],
    "INVALID_DESCRIPTOR",
  );
  const source = validateSourceContract(
    registration.source as NativeAdapterSourceContractInput,
    registration.sourceMaterials as ReadonlyArray<SourceMaterialInput>,
  );
  if (
    source.adapterId !== registration.driverId ||
    source.adapterVersion !== registration.adapterVersion
  ) {
    throw new NativeAdapterError(
      "INVALID_DESCRIPTOR",
      "source ledger identity must match the registered driver identity",
    );
  }
  if (
    typeof registration.driverId !== "string" ||
    registration.driverId.length === 0 ||
    registration.driverId !== registration.driverId.trim()
  ) {
    throw new NativeAdapterError("INVALID_DESCRIPTOR", "driverId must be canonical and non-empty");
  }
  if (
    typeof registration.adapterVersion !== "string" ||
    registration.adapterVersion.length === 0 ||
    registration.adapterVersion !== registration.adapterVersion.trim()
  ) {
    throw new NativeAdapterError(
      "INVALID_DESCRIPTOR",
      "adapterVersion must be canonical and non-empty",
    );
  }
  const rawTransports = snapshotPassiveArray(
    registration.transports,
    "registration.transports",
    "INVALID_DESCRIPTOR",
  );
  if (rawTransports.length === 0) {
    throw new NativeAdapterError("INVALID_DESCRIPTOR", "at least one transport is required");
  }
  const transports = [...new Set(rawTransports)] as NativeTransportKind[];
  if (transports.some((item) => item !== "process" && item !== "sdk")) {
    throw new NativeAdapterError("INVALID_DESCRIPTOR", "transport kind is unsupported");
  }
  return Object.freeze({
    driverId: registration.driverId as string,
    adapterVersion: registration.adapterVersion as string,
    source,
    transports: Object.freeze(transports),
  });
}

/** A registry is a source ledger and lookup table, not an account or process authority. */
export class NativeAdapterRegistry {
  readonly #descriptors = new Map<string, Readonly<NativeAdapterDescriptor>>();

  constructor(registrations: ReadonlyArray<NativeAdapterRegistration> = []) {
    const entries = snapshotPassiveArray(registrations, "registrations", "INVALID_DESCRIPTOR");
    for (const registration of entries) this.register(registration as NativeAdapterRegistration);
  }

  register(registration: NativeAdapterRegistration): Readonly<NativeAdapterDescriptor> {
    const descriptor = canonicalDescriptor(registration);
    const descriptorKey = key(descriptor.driverId, descriptor.adapterVersion);
    if (this.#descriptors.has(descriptorKey)) {
      throw new NativeAdapterError("INVALID_DESCRIPTOR", "driver/version is already registered");
    }
    this.#descriptors.set(descriptorKey, descriptor);
    return descriptor;
  }

  resolve(driverId: string, adapterVersion: string): Readonly<NativeAdapterDescriptor> | undefined {
    return this.#descriptors.get(key(driverId, adapterVersion));
  }

  require(driverId: string, adapterVersion: string): Readonly<NativeAdapterDescriptor> {
    const descriptor = this.resolve(driverId, adapterVersion);
    if (descriptor === undefined) {
      const sameDriver = [...this.#descriptors.values()].some((item) => item.driverId === driverId);
      throw new NativeAdapterError(
        sameDriver ? "DRIVER_VERSION_MISMATCH" : "UNKNOWN_DRIVER",
        sameDriver
          ? `driver ${driverId} has no version ${adapterVersion}`
          : `driver ${driverId} is unknown`,
      );
    }
    return descriptor;
  }

  list(): ReadonlyArray<Readonly<NativeAdapterDescriptor>> {
    return Object.freeze([...this.#descriptors.values()]);
  }
}

function normalizedFailure(error: unknown, operation: string): NativeAdapterError {
  if (error instanceof NativeAdapterError) return error;
  return new NativeAdapterError("PORT_PROTOCOL_ERROR", `${operation} failed`, error);
}

function snapshotCloseReceipt(value: NativeCloseReceipt): Readonly<NativeCloseReceipt> {
  const record = snapshotPassiveRecord(
    value,
    "closeReceipt",
    ["status", "reason", "binding", "custodyRef", "processIdentity"],
    "PORT_PROTOCOL_ERROR",
  );
  if (record.status !== "closed" && record.status !== "already-closed") {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "close receipt status is invalid");
  }
  if (
    typeof record.reason !== "string" ||
    record.reason.length === 0 ||
    record.reason.includes("\0")
  ) {
    throw new NativeAdapterError("PORT_PROTOCOL_ERROR", "close receipt reason is invalid");
  }
  const binding = validateBinding(record.binding as NativeAdapterBinding);
  const custodyRef = record.custodyRef;
  if (
    typeof custodyRef !== "string" ||
    custodyRef.length === 0 ||
    custodyRef !== custodyRef.trim()
  ) {
    throw new NativeAdapterError(
      "PORT_PROTOCOL_ERROR",
      "close receipt custody reference is invalid",
    );
  }
  const processIdentity =
    record.processIdentity === null
      ? null
      : validateProcessIdentity(record.processIdentity as ProcessIdentity);
  return Object.freeze({
    status: record.status,
    reason: record.reason,
    binding,
    custodyRef,
    processIdentity,
  });
}

class NativeAdapterSessionImpl implements NativeAdapterSession {
  readonly driverId: string;
  readonly adapterVersion: string;
  readonly source: Readonly<NativeAdapterSourceContract>;
  readonly binding: Readonly<NativeAdapterBinding>;
  readonly custodyRef: string;
  readonly instanceKey: string;
  readonly processIdentity: ProcessIdentity | null;
  #state: NativeSessionState = "active";
  #lastError: NativeAdapterError | undefined;
  #closeReceipt: NativeCloseReceipt | undefined;
  #closePromise: Promise<NativeCloseReceipt> | undefined;
  readonly #port: ActiveBytePort;
  readonly #maxFrameBytes: number;

  constructor(
    descriptor: Readonly<NativeAdapterDescriptor>,
    binding: Readonly<NativeAdapterBinding>,
    port: ActiveBytePort,
    maxFrameBytes: number,
  ) {
    this.driverId = descriptor.driverId;
    this.adapterVersion = descriptor.adapterVersion;
    this.source = descriptor.source;
    this.binding = binding;
    this.custodyRef = port.custodyRef;
    this.instanceKey = bindingKey(binding);
    this.#port = port;
    this.processIdentity = port.processIdentity;
    this.#maxFrameBytes = maxFrameBytes;
  }

  get state(): NativeSessionState {
    return this.#state;
  }

  async send(bytes: ByteInput, context: NativeOperationContext): Promise<void> {
    try {
      this.#assertUsable(context);
      await this.#port.write(cloneBytes(bytes, "session.send", this.#maxFrameBytes), context);
    } catch (error) {
      const normalized = normalizedFailure(error, "session send");
      if (this.#state === "active") throw this.#recordFailure(normalized, "session send");
      throw normalized;
    }
  }

  async receive(context: NativeOperationContext): Promise<Readonly<NativeFrame>> {
    try {
      this.#assertUsable(context);
      const frame = cloneFrame(await this.#port.read(context), this.#maxFrameBytes);
      if (frame.kind === "eof") this.#state = "eof";
      if (frame.kind === "error" && frame.fatal) this.#state = "failed";
      return frame;
    } catch (error) {
      const normalized = normalizedFailure(error, "session receive");
      if (this.#state === "active") throw this.#recordFailure(normalized, "session receive");
      throw normalized;
    }
  }

  async close(reason: string, context: NativeOperationContext): Promise<NativeCloseReceipt> {
    assertOperationUsable(context);
    assertBindingMatches(this.binding, context.binding);
    if (context.custodyRef !== this.custodyRef) {
      throw new NativeAdapterError(
        "INVALID_LAUNCH",
        "close custody reference does not match the admitted session",
      );
    }
    if (this.#closeReceipt !== undefined) {
      return Object.freeze({ ...this.#closeReceipt, status: "already-closed" as const });
    }
    if (this.#closePromise !== undefined) return this.#closePromise;
    if (typeof reason !== "string" || reason.length === 0 || reason.includes("\0")) {
      throw new NativeAdapterError(
        "SESSION_STATE_ERROR",
        "close reason must be non-empty and NUL-free",
      );
    }
    this.#state = "closing";
    this.#closePromise = this.#port
      .close(reason, context)
      .then((receipt) => {
        const normalizedReceipt = snapshotCloseReceipt(receipt);
        if (
          bindingKey(normalizedReceipt.binding) !== bindingKey(this.binding) ||
          normalizedReceipt.custodyRef !== this.custodyRef ||
          !sameProcessIdentity(normalizedReceipt.processIdentity, this.processIdentity)
        ) {
          throw new NativeAdapterError(
            "PROCESS_IDENTITY_UNKNOWN",
            "close receipt does not match admitted custody",
            {
              receipt: normalizedReceipt,
              expected: {
                binding: this.binding,
                custodyRef: this.custodyRef,
                processIdentity: this.processIdentity,
              },
            },
          );
        }
        this.#closeReceipt = Object.freeze({ ...normalizedReceipt, status: "closed" as const });
        this.#state = "closed";
        return this.#closeReceipt;
      })
      .catch((error: unknown) => {
        this.#state = "failed";
        throw normalizedFailure(error, "session close");
      });
    return this.#closePromise;
  }

  #assertUsable(context: NativeOperationContext): void {
    assertOperationUsable(context);
    assertBindingMatches(this.binding, context.binding);
    if (this.#state !== "active") {
      throw (
        this.#lastError ??
        new NativeAdapterError("SESSION_STATE_ERROR", `session is ${this.#state}, not active`)
      );
    }
  }

  #recordFailure(error: unknown, operation: string): NativeAdapterError {
    const normalized = normalizedFailure(error, operation);
    this.#lastError = normalized;
    if (normalized.code === "CANCELLED" || normalized.code === "DEADLINE_EXCEEDED") {
      this.#state = "cancelled";
    } else if (normalized.code === "PORT_CLOSED") {
      this.#state = "closed";
    } else {
      this.#state = "failed";
    }
    return normalized;
  }
}

function sameProcessIdentity(left: ProcessIdentity | null, right: ProcessIdentity | null): boolean {
  if (left === null || right === null) return left === right;
  return left.processId === right.processId && left.creationTime === right.creationTime;
}

function validateMaxFrameBytes(value: number): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new NativeAdapterError("INVALID_FRAME", "maxFrameBytes must be a positive safe integer");
  }
  return value;
}

/** Open one instance without making the registry a global runtime owner. */
export async function openNativeAdapterSession(
  registry: NativeAdapterRegistry,
  request: OpenNativeAdapterRequest,
): Promise<NativeAdapterSession> {
  const descriptor = registry.require(request.driverId, request.adapterVersion);
  const binding = validateBinding(request.binding);
  assertBindingMatches(binding, request.context.binding);
  const maxFrameBytes = validateMaxFrameBytes(request.maxFrameBytes ?? 4 * 1024 * 1024);
  if (!descriptor.transports.includes(request.transport.kind)) {
    throw new NativeAdapterError(
      "INVALID_DESCRIPTOR",
      `driver ${descriptor.driverId} does not expose ${request.transport.kind} transport`,
    );
  }
  let port: ActiveBytePort;
  if (request.transport.kind === "process") {
    if (
      request.transport.launch.adapterId !== descriptor.driverId ||
      request.transport.launch.adapterVersion !== descriptor.adapterVersion
    ) {
      throw new NativeAdapterError(
        "DRIVER_VERSION_MISMATCH",
        "launch identity does not match the descriptor",
      );
    }
    port = await openManagedProcessPort(
      request.transport.port,
      request.transport.launch,
      request.context,
    );
  } else {
    port = openManagedSdkPort(request.transport.transport, request.context, maxFrameBytes);
  }
  return new NativeAdapterSessionImpl(descriptor, binding, port, maxFrameBytes);
}
