import type {
  JsonObject,
  JsonValue,
  RuntimeAccountRef,
  RuntimeDriverId,
  RuntimeInstanceId,
} from "../contracts/model.ts";
import {
  adapterIdentity,
  assertHostCompatibility,
  cloneAndFreezeJson,
  manifestRegistrationKey,
  parseAdapterManifest,
  RuntimeCatalogError,
} from "./manifest.ts";
import {
  RUNTIME_INSTANCE_CONFIG_SCHEMA,
  type AdapterIdentity,
  type AdapterRegistration,
  type DecodedRuntimeInstanceConfig,
  type ResolvedRuntimeInstanceConfig,
  type RuntimeInstanceConfig,
  type RuntimeInstanceLeaseState,
  type RuntimeCatalogHostContext,
} from "./types.ts";

const OPEN_ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const INSTANCE_CONFIG_FIELDS = new Set([
  "schema",
  "instanceId",
  "driverId",
  "adapterVersion",
  "enabled",
  "accountRef",
  "config",
]);

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const instanceField = (raw: Record<string, unknown>, field: string): string => {
  const value = raw[field];
  if (typeof value !== "string" || !OPEN_ID_PATTERN.test(value)) {
    throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", `${field} must be an open runtime id`);
  }
  return value;
};

const requiredString = (raw: Record<string, unknown>, field: string): string => {
  const value = raw[field];
  if (typeof value !== "string" || value.length === 0) {
    throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", `${field} must be a non-empty string`);
  }
  return value;
};

const cloneInstanceJson = (value: unknown, path: string): JsonValue => {
  try {
    return cloneAndFreezeJson(value as JsonValue, path);
  } catch (error) {
    if (error instanceof RuntimeCatalogError) {
      throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", error.message);
    }
    throw error;
  }
};

const leaseOwners = new WeakMap<object, object>();
const authenticDecodedInstances = new WeakSet<object>();

const freezeAdapterIdentity = (identity: AdapterIdentity | null): AdapterIdentity | null =>
  identity === null
    ? null
    : Object.freeze({
        driverId: identity.driverId,
        adapterVersion: identity.adapterVersion,
        artifactDigest: identity.artifactDigest,
      });

const registerLeaseState = (
  state: RuntimeInstanceLeaseState,
  owner: object,
): RuntimeInstanceLeaseState => {
  const frozen = Object.freeze({
    instanceId: state.instanceId,
    driverId: state.driverId,
    enabled: state.enabled,
    phase: state.phase,
    currentAdapter: freezeAdapterIdentity(state.currentAdapter),
    pendingAdapter: freezeAdapterIdentity(state.pendingAdapter),
    activeBindings: Object.freeze([...state.activeBindings]),
  });
  leaseOwners.set(frozen, owner);
  return frozen;
};

const leaseOwner = (state: RuntimeInstanceLeaseState, expectedOwner?: object): object => {
  const owner = leaseOwners.get(state);
  if (owner === undefined || (expectedOwner !== undefined && owner !== expectedOwner)) {
    throw new RuntimeCatalogError(
      "UNTRUSTED_LEASE_STATE",
      "Lease state must originate from the owning RuntimeCatalog and registered transitions",
    );
  }
  return owner;
};

export function decodeRuntimeInstanceConfig(input: unknown): DecodedRuntimeInstanceConfig {
  const snapshot = cloneInstanceJson(input, "instance");
  if (!isObject(snapshot)) {
    throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", "Instance config must be an object");
  }
  const raw = snapshot;
  if (raw.schema !== RUNTIME_INSTANCE_CONFIG_SCHEMA) {
    throw new RuntimeCatalogError(
      "INVALID_INSTANCE_CONFIG",
      `Unsupported instance config schema ${String(raw.schema)}`,
    );
  }
  if (raw.enabled !== undefined && typeof raw.enabled !== "boolean") {
    throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", "enabled must be a boolean");
  }
  if (raw.config === undefined) {
    throw new RuntimeCatalogError("INVALID_INSTANCE_CONFIG", "config must be present");
  }

  const unknown: Record<string, JsonValue> = {};
  for (const [field, value] of Object.entries(raw)) {
    if (!INSTANCE_CONFIG_FIELDS.has(field)) {
      unknown[field] = cloneInstanceJson(value, `instance.${field}`);
    }
  }
  const accountRef =
    raw.accountRef === undefined
      ? undefined
      : (instanceField(raw, "accountRef") as RuntimeAccountRef);
  const value: RuntimeInstanceConfig = {
    instanceId: instanceField(raw, "instanceId") as RuntimeInstanceId,
    driverId: instanceField(raw, "driverId") as RuntimeDriverId,
    adapterVersion: requiredString(raw, "adapterVersion"),
    ...(raw.enabled === undefined ? {} : { enabled: raw.enabled }),
    ...(accountRef === undefined ? {} : { accountRef }),
    config: cloneInstanceJson(raw.config, "instance.config"),
  };
  const decoded = cloneAndFreezeJson({
    value,
    unknownFields: unknown as JsonObject,
  } as unknown as JsonValue) as unknown as DecodedRuntimeInstanceConfig;
  authenticDecodedInstances.add(decoded);
  return decoded;
}

export const encodeRuntimeInstanceConfig = (decoded: DecodedRuntimeInstanceConfig): JsonObject => {
  if (!authenticDecodedInstances.has(decoded)) {
    throw new RuntimeCatalogError(
      "UNTRUSTED_INSTANCE_CONFIG",
      "Runtime instance config must originate from decodeRuntimeInstanceConfig",
    );
  }
  return cloneAndFreezeJson({
    ...decoded.unknownFields,
    schema: RUNTIME_INSTANCE_CONFIG_SCHEMA,
    instanceId: decoded.value.instanceId,
    driverId: decoded.value.driverId,
    adapterVersion: decoded.value.adapterVersion,
    ...(decoded.value.enabled === undefined ? {} : { enabled: decoded.value.enabled }),
    ...(decoded.value.accountRef === undefined ? {} : { accountRef: decoded.value.accountRef }),
    config: decoded.value.config,
  });
};

export class RuntimeCatalog {
  readonly #registrations: ReadonlyMap<string, AdapterRegistration>;
  readonly #resolvedInstances = new WeakSet<object>();
  readonly #leaseOwner = Object.freeze({});

  constructor(
    registrations: ReadonlyArray<AdapterRegistration>,
    options: RuntimeCatalogHostContext,
  ) {
    const byKey = new Map<string, AdapterRegistration>();
    const registrationSnapshot = cloneAndFreezeJson(
      registrations as unknown as JsonValue,
      "registrations",
    );
    if (!Array.isArray(registrationSnapshot)) {
      throw new RuntimeCatalogError("INVALID_MANIFEST", "registrations must be an array");
    }
    for (const registration of registrationSnapshot as unknown as ReadonlyArray<AdapterRegistration>) {
      const manifest = parseAdapterManifest(registration.manifest);
      assertHostCompatibility(manifest, options);
      const key = manifestRegistrationKey(manifest.driverId, manifest.adapterVersion);
      if (byKey.has(key)) {
        throw new RuntimeCatalogError("DUPLICATE_REGISTRATION", `Duplicate adapter ${key}`);
      }
      byKey.set(key, Object.freeze({ manifest }));
    }
    this.#registrations = byKey;
  }

  resolveRegistration(
    driverId: RuntimeDriverId,
    adapterVersion: string,
  ): AdapterRegistration | null {
    return this.#registrations.get(manifestRegistrationKey(driverId, adapterVersion)) ?? null;
  }

  resolveInstance(decoded: DecodedRuntimeInstanceConfig): ResolvedRuntimeInstanceConfig {
    if (!authenticDecodedInstances.has(decoded)) {
      throw new RuntimeCatalogError(
        "UNTRUSTED_INSTANCE_CONFIG",
        "Runtime instance config must originate from decodeRuntimeInstanceConfig",
      );
    }
    const exact = this.resolveRegistration(decoded.value.driverId, decoded.value.adapterVersion);
    if (exact !== null) {
      const resolved = Object.freeze({
        decoded,
        enabled: decoded.value.enabled ?? false,
        availability: Object.freeze({ status: "available" as const, registration: exact }),
      });
      this.#resolvedInstances.add(resolved);
      return resolved;
    }
    const hasDriver = [...this.#registrations.values()].some(
      (entry) => entry.manifest.driverId === decoded.value.driverId,
    );
    const resolved = Object.freeze({
      decoded,
      enabled: decoded.value.enabled ?? false,
      availability: Object.freeze({
        status: "unavailable" as const,
        reason: hasDriver ? "ADAPTER_VERSION_NOT_REGISTERED" : "DRIVER_NOT_REGISTERED",
      }),
    });
    this.#resolvedInstances.add(resolved);
    return resolved;
  }

  initialLeaseState(resolved: ResolvedRuntimeInstanceConfig): RuntimeInstanceLeaseState {
    if (!this.#resolvedInstances.has(resolved)) {
      throw new RuntimeCatalogError(
        "UNTRUSTED_RESOLVED_INSTANCE",
        "Resolved runtime instance must originate from this module's RuntimeCatalog.resolveInstance",
      );
    }
    const registration =
      resolved.availability.status === "available" ? resolved.availability.registration : null;
    return registerLeaseState(
      {
        instanceId: resolved.decoded.value.instanceId,
        driverId: resolved.decoded.value.driverId,
        enabled: resolved.enabled,
        phase: registration === null ? "unavailable" : resolved.enabled ? "ready" : "disabled",
        currentAdapter: registration === null ? null : adapterIdentity(registration.manifest),
        pendingAdapter: null,
        activeBindings: [],
      },
      this.#leaseOwner,
    );
  }

  requestAdapterSwitch(
    state: RuntimeInstanceLeaseState,
    adapterVersion: string,
  ): RuntimeInstanceLeaseState {
    leaseOwner(state, this.#leaseOwner);
    const currentState = state;
    const registration = this.resolveRegistration(currentState.driverId, adapterVersion);
    if (registration === null) {
      throw new RuntimeCatalogError(
        "SWITCH_NOT_AVAILABLE",
        `${currentState.driverId}@${adapterVersion} is not registered in this build`,
      );
    }
    const target = adapterIdentity(registration.manifest);
    if (sameAdapter(currentState.currentAdapter, target)) return currentState;
    if (currentState.phase === "draining") {
      if (sameAdapter(currentState.pendingAdapter, target)) return currentState;
      throw new RuntimeCatalogError(
        "SWITCH_NOT_AVAILABLE",
        `Instance ${currentState.instanceId} is already draining to another adapter`,
      );
    }
    if (currentState.activeBindings.length > 0) {
      return registerLeaseState(
        {
          ...currentState,
          phase: "draining",
          pendingAdapter: target,
        },
        this.#leaseOwner,
      );
    }
    return registerLeaseState(
      {
        ...currentState,
        phase: currentState.enabled ? "ready" : "disabled",
        currentAdapter: target,
        pendingAdapter: null,
      },
      this.#leaseOwner,
    );
  }
}

const sameAdapter = (left: AdapterIdentity | null, right: AdapterIdentity): boolean =>
  left !== null &&
  left.driverId === right.driverId &&
  left.adapterVersion === right.adapterVersion &&
  left.artifactDigest === right.artifactDigest;

export const openBinding = (
  state: RuntimeInstanceLeaseState,
  bindingId: string,
): RuntimeInstanceLeaseState => {
  const owner = leaseOwner(state);
  const currentState = state;
  if (currentState.phase !== "ready") {
    throw new RuntimeCatalogError(
      "SWITCH_NOT_AVAILABLE",
      `Instance ${currentState.instanceId} cannot accept bindings while ${currentState.phase}`,
    );
  }
  if (currentState.activeBindings.includes(bindingId)) return currentState;
  return registerLeaseState(
    {
      ...currentState,
      activeBindings: [...currentState.activeBindings, bindingId],
    },
    owner,
  );
};

export const closeBinding = (
  state: RuntimeInstanceLeaseState,
  bindingId: string,
): RuntimeInstanceLeaseState => {
  const owner = leaseOwner(state);
  const currentState = state;
  const activeBindings = currentState.activeBindings.filter((entry) => entry !== bindingId);
  if (
    currentState.phase === "draining" &&
    activeBindings.length === 0 &&
    currentState.pendingAdapter !== null
  ) {
    return registerLeaseState(
      {
        ...currentState,
        activeBindings,
        currentAdapter: currentState.pendingAdapter,
        pendingAdapter: null,
        phase: currentState.enabled ? "ready" : "disabled",
      },
      owner,
    );
  }
  return registerLeaseState({ ...currentState, activeBindings }, owner);
};
