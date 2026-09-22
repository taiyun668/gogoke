import * as NodeUtilTypes from "node:util/types";

export type HostAuthority = "legacy" | "public";

export interface HostBinding {
  readonly rootIdentity: string;
  readonly profileId: string;
  readonly runtimeInstanceId: string;
  readonly authRevision: string;
  readonly generation: string;
}

export interface HostOperationContext {
  readonly signal: AbortSignal;
  readonly deadlineAt: number;
}

export interface PreparedLaunch {
  readonly ticketId: string;
  readonly custodyRef: string;
  readonly processIdentity: { readonly processId: number; readonly creationTime: string } | null;
}

export interface RuntimeExit {
  readonly kind: "clean" | "crash" | "abnormal";
  readonly code: number | null;
}

export interface RuntimeChannel {
  authenticate(binding: HostBinding, context: HostOperationContext): Promise<void>;
  subscribe(binding: HostBinding, context: HostOperationContext): Promise<void>;
  initialize(binding: HostBinding, context: HostOperationContext): Promise<void>;
  close(reason: string, context: HostOperationContext): Promise<void>;
  readonly exited?: Promise<RuntimeExit>;
}

export interface NativeHostPort {
  prepare(binding: HostBinding, context: HostOperationContext): Promise<PreparedLaunch>;
  activate(ticket: PreparedLaunch, context: HostOperationContext): Promise<void>;
  abortPrepared(candidate: unknown, reason: string, context: HostOperationContext): Promise<void>;
}

export type OwnerReconciliation = "committed" | "not-committed" | "unknown";

export interface DurableHostStore {
  /** Resolves only after the owner and launch custody are durably committed. */
  persistOwner(input: DurableOwnerInput, context: HostOperationContext): Promise<void>;
  /** Reconciles a persist whose completion raced timeout/abort. */
  reconcileOwner(
    input: DurableOwnerInput,
    context: HostOperationContext,
  ): Promise<OwnerReconciliation>;
  /** Must retain the prepared launch even when normal persistence was unknown. */
  retainStartupCustody(input: DurableCustodyInput, context: HostOperationContext): Promise<void>;
}

export interface DurableOwnerInput {
  readonly authority: HostAuthority;
  readonly binding: HostBinding;
  readonly sourceEpoch: string;
  readonly launch: PreparedLaunch;
}

export interface DurableCustodyInput extends DurableOwnerInput {
  readonly reason: string;
}

export interface RuntimeConnector {
  connect(
    binding: HostBinding,
    launch: PreparedLaunch,
    context: HostOperationContext,
  ): Promise<RuntimeChannel>;
}

export interface HostAdapters {
  readonly nativeHost: NativeHostPort;
  readonly store: DurableHostStore;
  readonly connector: RuntimeConnector;
}

export type HostErrorCode =
  | "INVALID_BINDING"
  | "INVALID_PREPARED_LAUNCH"
  | "ROOT_PROFILE_ALREADY_OWNED"
  | "FAILED_CUSTODY"
  | "PREPARE_OUTCOME_UNKNOWN"
  | "UNKNOWN_PROCESS_IDENTITY"
  | "OPERATION_TIMEOUT"
  | "PRODUCTION_ADAPTER_UNAVAILABLE";

export class HostCoordinatorError extends Error {
  override readonly name = "HostCoordinatorError";
  readonly code: HostErrorCode;
  override readonly cause: unknown;

  constructor(code: HostErrorCode, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.cause = cause;
  }
}

export interface ReadyHost {
  readonly authority: HostAuthority;
  readonly binding: HostBinding;
  readonly sourceEpoch: string;
  readonly custodyRef: string;
}

export type HostClaimState = "starting" | "ready" | "startup-custody" | "prepare-outcome-unknown";

interface RootClaim {
  readonly authority: HostAuthority;
  readonly instanceKey: string;
  state: HostClaimState;
}

interface EpochFlight<T> {
  readonly epoch: string;
  readonly promise: Promise<T>;
}

type OperationPhase =
  | "prepare"
  | "persist-owner"
  | "activate"
  | "connect"
  | "authenticate"
  | "subscribe"
  | "initialize";

interface OwnedOperationOptions<T> {
  readonly phase: string;
  readonly start: (context: HostOperationContext) => Promise<T>;
  readonly onLateSuccess?: (value: T) => void | Promise<void>;
  readonly onLateFailure?: (error: unknown) => void | Promise<void>;
}

const U64_PATTERN = /^(?:0|[1-9]\d*)$/;
const U64_MAX = 18_446_744_073_709_551_615n;

function invalidLaunch(path: string, detail: string): never {
  throw new HostCoordinatorError("INVALID_PREPARED_LAUNCH", `${path} ${detail}`);
}

function passiveRecord(
  value: unknown,
  path: string,
  expectedKeys: ReadonlyArray<string>,
): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return invalidLaunch(path, "must be a non-Proxy plain object");
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return invalidLaunch(path, "must not contain symbol keys");
  }
  const names = keys as ReadonlyArray<string>;
  const extra = names.find((name) => !expectedKeys.includes(name));
  if (extra !== undefined) return invalidLaunch(`${path}.${extra}`, "is not allowed");
  const missing = expectedKeys.find((name) => !names.includes(name));
  if (missing !== undefined) return invalidLaunch(`${path}.${missing}`, "is required");
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const snapshot: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return invalidLaunch(`${path}.${name}`, "must be an enumerable data property");
    }
    snapshot[name] = descriptor.value;
  }
  return snapshot;
}

function canonicalString(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return invalidLaunch(path, "must be canonical and non-empty");
  }
  return value;
}

export function snapshotPreparedLaunch(value: unknown): PreparedLaunch {
  const launch = passiveRecord(value, "launch", ["ticketId", "custodyRef", "processIdentity"]);
  const ticketId = canonicalString(launch.ticketId, "launch.ticketId");
  const custodyRef = canonicalString(launch.custodyRef, "launch.custodyRef");
  let processIdentity: PreparedLaunch["processIdentity"];
  if (launch.processIdentity === null) {
    processIdentity = null;
  } else {
    const identity = passiveRecord(launch.processIdentity, "launch.processIdentity", [
      "processId",
      "creationTime",
    ]);
    if (!Number.isSafeInteger(identity.processId) || (identity.processId as number) <= 0) {
      return invalidLaunch("launch.processIdentity.processId", "must be a positive safe integer");
    }
    const creationTime = canonicalString(
      identity.creationTime,
      "launch.processIdentity.creationTime",
    );
    if (!U64_PATTERN.test(creationTime) || BigInt(creationTime) > U64_MAX) {
      return invalidLaunch("launch.processIdentity.creationTime", "must be a canonical u64");
    }
    processIdentity = Object.freeze({
      processId: identity.processId as number,
      creationTime,
    });
  }
  return Object.freeze({ ticketId, custodyRef, processIdentity });
}

function snapshotBinding(value: HostBinding): HostBinding {
  const labels = ["rootIdentity", "profileId", "runtimeInstanceId"] as const;
  for (const label of labels) {
    const field = value[label];
    if (typeof field !== "string" || field.length === 0 || field !== field.trim()) {
      throw new HostCoordinatorError("INVALID_BINDING", `${label} must be canonical and non-empty`);
    }
  }
  for (const label of ["authRevision", "generation"] as const) {
    const field = value[label];
    if (!U64_PATTERN.test(field) || BigInt(field) > U64_MAX) {
      throw new HostCoordinatorError("INVALID_BINDING", `${label} must be a canonical u64 string`);
    }
  }
  return Object.freeze({ ...value });
}

export const rootProfileKey = (binding: HostBinding): string =>
  JSON.stringify([binding.rootIdentity, binding.profileId]);

export const runtimeInstanceKey = (binding: HostBinding): string =>
  JSON.stringify([
    binding.rootIdentity,
    binding.profileId,
    binding.runtimeInstanceId,
    binding.authRevision,
    binding.generation,
  ]);

export class EpochSingleflight<T> {
  readonly #flights = new Map<string, EpochFlight<T>>();

  join(key: string, epoch: string, construct: () => Promise<T>): Promise<T> {
    const existing = this.#flights.get(key);
    if (existing?.epoch === epoch) return existing.promise;
    if (existing !== undefined) {
      throw new HostCoordinatorError(
        "FAILED_CUSTODY",
        `connection epoch ${existing.epoch} is still current for ${key}`,
      );
    }
    return this.replace(key, epoch, construct);
  }

  replace(key: string, epoch: string, construct: () => Promise<T>): Promise<T> {
    const promise = Promise.resolve().then(construct);
    const flight = { epoch, promise };
    this.#flights.set(key, flight);
    void promise.then(
      () => this.#compareClear(key, flight),
      () => this.#compareClear(key, flight),
    );
    return promise;
  }

  currentEpoch(key: string): string | undefined {
    return this.#flights.get(key)?.epoch;
  }

  #compareClear(key: string, flight: EpochFlight<T>): void {
    if (this.#flights.get(key) === flight) this.#flights.delete(key);
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export class HostCoordinator {
  readonly #adapters: HostAdapters;
  readonly #operationTimeoutMs: number;
  readonly #claims = new Map<string, RootClaim>();
  readonly #startFlights = new Map<string, Promise<ReadyHost>>();
  readonly #readyHosts = new Map<string, ReadyHost>();
  readonly #connections = new EpochSingleflight<RuntimeChannel>();
  readonly #closedChannels = new WeakSet<RuntimeChannel>();
  readonly #abortedLaunches = new WeakSet<object>();
  #nextEpoch = 0n;

  constructor(adapters: HostAdapters, operationTimeoutMs = 30_000) {
    this.#adapters = adapters;
    this.#operationTimeoutMs = operationTimeoutMs;
    if (!Number.isSafeInteger(operationTimeoutMs) || operationTimeoutMs <= 0) {
      throw new HostCoordinatorError("INVALID_BINDING", "operation timeout must be positive");
    }
  }

  start(authority: HostAuthority, requestedBinding: HostBinding): Promise<ReadyHost> {
    if (authority !== "legacy" && authority !== "public") {
      return Promise.reject(
        new HostCoordinatorError("INVALID_BINDING", "authority must be legacy or public"),
      );
    }
    const binding = snapshotBinding(requestedBinding);
    const rootKey = rootProfileKey(binding);
    const instanceKey = runtimeInstanceKey(binding);
    const existingClaim = this.#claims.get(rootKey);
    if (existingClaim !== undefined) {
      const sameOwner =
        existingClaim.authority === authority && existingClaim.instanceKey === instanceKey;
      if (!sameOwner) {
        return Promise.reject(
          new HostCoordinatorError(
            "ROOT_PROFILE_ALREADY_OWNED",
            `${existingClaim.authority} already admitted this root/profile; no alternate spawn`,
          ),
        );
      }
      if (
        existingClaim.state === "startup-custody" ||
        existingClaim.state === "prepare-outcome-unknown"
      ) {
        return Promise.reject(
          new HostCoordinatorError(
            "FAILED_CUSTODY",
            `${existingClaim.state} requires explicit Controller reconciliation`,
          ),
        );
      }
      if (existingClaim.state === "ready") {
        const ready = this.#readyHosts.get(instanceKey);
        if (ready === undefined) {
          return Promise.reject(
            new HostCoordinatorError("FAILED_CUSTODY", "ready admission has no host record"),
          );
        }
        return Promise.resolve(ready);
      }
    } else {
      this.#claims.set(rootKey, { authority, instanceKey, state: "starting" });
    }
    const existingFlight = this.#startFlights.get(instanceKey);
    if (existingFlight !== undefined) return existingFlight;
    const sourceEpoch = (++this.#nextEpoch).toString();
    const flight = this.#start(authority, binding, sourceEpoch, rootKey, instanceKey);
    this.#startFlights.set(instanceKey, flight);
    void flight
      .finally(() => {
        if (this.#startFlights.get(instanceKey) === flight) this.#startFlights.delete(instanceKey);
      })
      .catch(() => undefined);
    return flight;
  }

  state(binding: HostBinding): HostClaimState | "absent" {
    return this.#claims.get(rootProfileKey(binding))?.state ?? "absent";
  }

  /** Shared Controller wiring must call this only after external custody reconciliation. */
  reconcileFailedAdmission(authority: HostAuthority, requestedBinding: HostBinding): void {
    const binding = snapshotBinding(requestedBinding);
    const rootKey = rootProfileKey(binding);
    const instanceKey = runtimeInstanceKey(binding);
    const claim = this.#claims.get(rootKey);
    if (
      claim === undefined ||
      claim.authority !== authority ||
      claim.instanceKey !== instanceKey ||
      (claim.state !== "startup-custody" && claim.state !== "prepare-outcome-unknown")
    ) {
      throw new HostCoordinatorError(
        "FAILED_CUSTODY",
        "no matching failed admission is eligible for reconciliation",
      );
    }
    this.#readyHosts.delete(instanceKey);
    this.#claims.delete(rootKey);
  }

  async #start(
    authority: HostAuthority,
    binding: HostBinding,
    sourceEpoch: string,
    rootKey: string,
    instanceKey: string,
  ): Promise<ReadyHost> {
    let phase: OperationPhase = "prepare";
    let launch: PreparedLaunch | undefined;
    let channel: RuntimeChannel | undefined;
    const ownerInput = (): DurableOwnerInput => ({
      authority,
      binding,
      sourceEpoch,
      launch: launch!,
    });
    try {
      const prepared = await this.#runOwnedOperation({
        phase,
        start: (context) => this.#adapters.nativeHost.prepare(binding, context),
        onLateSuccess: async (late) => {
          try {
            const snapshot = snapshotPreparedLaunch(late);
            this.#scheduleRecovery({
              owner: { authority, binding, sourceEpoch, launch: snapshot },
              reason: "prepare completed after deadline",
              reconcileOwner: false,
            });
          } catch {
            await this.#abortPreparedOnce(late, "invalid prepare completed after deadline");
          }
        },
      });
      try {
        launch = snapshotPreparedLaunch(prepared);
      } catch (error) {
        void this.#abortPreparedOnce(prepared, "prepare returned an invalid launch");
        throw error;
      }

      phase = "persist-owner";
      await this.#runOwnedOperation({
        phase,
        start: (context) => this.#adapters.store.persistOwner(ownerInput(), context),
        onLateSuccess: () =>
          this.#scheduleRecovery({
            owner: ownerInput(),
            reason: "persist owner completed after deadline",
            reconcileOwner: true,
          }),
        onLateFailure: () =>
          this.#scheduleRecovery({
            owner: ownerInput(),
            reason: "persist owner failed after deadline",
            reconcileOwner: true,
          }),
      });
      if (launch.processIdentity === null) {
        throw new HostCoordinatorError(
          "UNKNOWN_PROCESS_IDENTITY",
          "prepared launch identity is unknown; retaining startup custody",
        );
      }

      phase = "activate";
      await this.#runOwnedOperation({
        phase,
        start: (context) => this.#adapters.nativeHost.activate(launch!, context),
        onLateSuccess: () => this.#abortPreparedOnce(launch!, "activate completed after deadline"),
      });

      phase = "connect";
      channel = await this.#connections.join(instanceKey, sourceEpoch, () =>
        this.#runOwnedOperation({
          phase,
          start: (context) => this.#adapters.connector.connect(binding, launch!, context),
          onLateSuccess: (late) => this.#closeChannelOnce(late, "connect completed after deadline"),
        }),
      );

      phase = "authenticate";
      await this.#runOwnedOperation({
        phase,
        start: (context) => channel!.authenticate(binding, context),
      });
      phase = "subscribe";
      await this.#runOwnedOperation({
        phase,
        start: (context) => channel!.subscribe(binding, context),
      });
      phase = "initialize";
      await this.#runOwnedOperation({
        phase,
        start: (context) => channel!.initialize(binding, context),
      });

      const claim = this.#claims.get(rootKey);
      if (
        claim === undefined ||
        claim.instanceKey !== instanceKey ||
        claim.authority !== authority
      ) {
        throw new HostCoordinatorError("FAILED_CUSTODY", "admission changed during initialize");
      }
      claim.state = "ready";
      this.#observeExit(authority, binding, sourceEpoch, launch, channel, claim);
      const ready = Object.freeze({
        authority,
        binding,
        sourceEpoch,
        custodyRef: launch.custodyRef,
      });
      this.#readyHosts.set(instanceKey, ready);
      return ready;
    } catch (error) {
      const claim = this.#claims.get(rootKey);
      if (launch === undefined) {
        if (claim !== undefined) claim.state = "prepare-outcome-unknown";
        throw new HostCoordinatorError(
          "PREPARE_OUTCOME_UNKNOWN",
          `prepare did not yield a trusted launch: ${messageOf(error)}`,
          error,
        );
      }
      if (claim !== undefined) claim.state = "startup-custody";
      if (channel !== undefined)
        this.#closeChannelOnce(channel, `startup failed: ${messageOf(error)}`);
      this.#scheduleRecovery({
        owner: ownerInput(),
        reason: `${phase} failed: ${messageOf(error)}`,
        reconcileOwner: phase === "persist-owner",
      });
      throw error;
    }
  }

  #runOwnedOperation<T>(options: OwnedOperationOptions<T>): Promise<T> {
    const controller = new AbortController();
    const deadlineAt = Date.now() + this.#operationTimeoutMs;
    const context = Object.freeze({ signal: controller.signal, deadlineAt });
    let timedOut = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const operation = Promise.resolve().then(() => options.start(context));
    void operation.then(
      (value) => {
        if (timedOut && options.onLateSuccess !== undefined) {
          void Promise.resolve(options.onLateSuccess(value)).catch(() => undefined);
        }
      },
      (error) => {
        if (timedOut && options.onLateFailure !== undefined) {
          void Promise.resolve(options.onLateFailure(error)).catch(() => undefined);
        }
      },
    );
    const timeout = new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => {
        timedOut = true;
        const error = new HostCoordinatorError(
          "OPERATION_TIMEOUT",
          `${options.phase} exceeded ${this.#operationTimeoutMs}ms`,
        );
        controller.abort(error);
        reject(error);
      }, this.#operationTimeoutMs);
    });
    return Promise.race([operation, timeout]).finally(() => {
      if (!timedOut && timer !== undefined) clearTimeout(timer);
    });
  }

  #scheduleRecovery(input: {
    readonly owner: DurableOwnerInput;
    readonly reason: string;
    readonly reconcileOwner: boolean;
  }): void {
    void (async () => {
      let reconciliation: OwnerReconciliation | undefined;
      if (input.reconcileOwner) {
        try {
          reconciliation = await this.#runOwnedOperation({
            phase: "reconcile-owner",
            start: (context) => this.#adapters.store.reconcileOwner(input.owner, context),
          });
        } catch {
          reconciliation = "unknown";
        }
      }
      const reason =
        reconciliation === undefined
          ? input.reason
          : `${input.reason}; persist reconciliation=${reconciliation}`;
      try {
        await this.#runOwnedOperation({
          phase: "retain-startup-custody",
          start: (context) =>
            this.#adapters.store.retainStartupCustody({ ...input.owner, reason }, context),
        });
      } catch {
        // The in-memory claim remains fail-closed even when durable recovery is unavailable.
      }
      await this.#abortPreparedOnce(input.owner.launch, reason);
    })().catch(() => undefined);
  }

  #abortPreparedOnce(candidate: unknown, reason: string): Promise<void> {
    if (typeof candidate === "object" && candidate !== null) {
      if (this.#abortedLaunches.has(candidate)) return Promise.resolve();
      this.#abortedLaunches.add(candidate);
    }
    return this.#runOwnedOperation({
      phase: "abort-prepared",
      start: (context) => this.#adapters.nativeHost.abortPrepared(candidate, reason, context),
    }).catch(() => undefined);
  }

  #closeChannelOnce(channel: RuntimeChannel, reason: string): Promise<void> {
    if (this.#closedChannels.has(channel)) return Promise.resolve();
    this.#closedChannels.add(channel);
    return this.#runOwnedOperation({
      phase: "close-late-channel",
      start: (context) => channel.close(reason, context),
    }).catch(() => undefined);
  }

  #observeExit(
    authority: HostAuthority,
    binding: HostBinding,
    sourceEpoch: string,
    launch: PreparedLaunch,
    channel: RuntimeChannel,
    claim: RootClaim,
  ): void {
    if (channel.exited === undefined) return;
    void channel.exited
      .then(
        (exit) => {
          this.#readyHosts.delete(runtimeInstanceKey(binding));
          claim.state = "startup-custody";
          this.#scheduleRecovery({
            owner: { authority, binding, sourceEpoch, launch },
            reason: `runtime ${exit.kind} exit (${exit.code ?? "unknown"})`,
            reconcileOwner: false,
          });
        },
        (error) => {
          this.#readyHosts.delete(runtimeInstanceKey(binding));
          claim.state = "startup-custody";
          this.#scheduleRecovery({
            owner: { authority, binding, sourceEpoch, launch },
            reason: `runtime exit observation failed: ${messageOf(error)}`,
            reconcileOwner: false,
          });
        },
      )
      .catch(() => undefined);
  }
}

export function createProductionHostCoordinator(): never {
  throw new HostCoordinatorError(
    "PRODUCTION_ADAPTER_UNAVAILABLE",
    "production DB/VFS/native-host adapters are not wired; readiness is blocked",
  );
}
