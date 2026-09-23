import * as NodeUtilTypes from "node:util/types";

import { LOCAL_NON_MODEL_CAPABILITY, validateMinimalReleaseRequest } from "../releasePolicy.ts";
import {
  RootProfileOwnership,
  type RootProfileOwnershipLease,
  type RootProfileOwnershipRequest,
} from "./rootProfileOwnership.ts";

export interface MinimalServiceConstructionRequest extends RootProfileOwnershipRequest {
  readonly requestedCapabilities?: readonly string[];
  readonly enabledRuntimeDriverIds?: readonly string[];
}

export interface ConstructedMinimalService<Service> {
  readonly service: Service;
  readonly ownership: RootProfileOwnershipLease;
}

type MinimalServiceInput<Service> = {
  readonly request: MinimalServiceConstructionRequest;
  readonly ownership: RootProfileOwnership;
  readonly constructLocalNonModelService: () => Service;
};

const invalidInput = (path: string, detail: string): never => {
  throw new Error(`INVALID_MINIMAL_SERVICE_INPUT: ${path} ${detail}`);
};

const snapshotRecord = (
  value: unknown,
  path: string,
  required: readonly string[],
  optional: readonly string[] = [],
): Readonly<Record<string, unknown>> => {
  if (
    typeof value !== "object" ||
    value === null ||
    NodeUtilTypes.isProxy(value) ||
    Array.isArray(value)
  ) {
    return invalidInput(path, "must be a non-Proxy plain object");
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    return invalidInput(path, "must be a non-Proxy plain object");
  }

  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return invalidInput(path, "must not contain symbol keys");
  }
  const names = keys as readonly string[];
  const allowed = new Set([...required, ...optional]);
  const extra = names.find((name) => !allowed.has(name));
  if (extra !== undefined) return invalidInput(`${path}.${extra}`, "is not allowed");
  const missing = required.find((name) => !names.includes(name));
  if (missing !== undefined) return invalidInput(`${path}.${missing}`, "is required");

  const descriptors = Object.getOwnPropertyDescriptors(value);
  const snapshot: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || descriptor.enumerable !== true) {
      return invalidInput(`${path}.${name}`, "must be an enumerable data property");
    }
    snapshot[name] = descriptor.value;
  }
  return Object.freeze(snapshot);
};

const snapshotStringArray = (value: unknown, path: string): readonly string[] => {
  if (
    typeof value !== "object" ||
    value === null ||
    NodeUtilTypes.isProxy(value) ||
    !Array.isArray(value)
  ) {
    return invalidInput(path, "must be a non-Proxy plain array");
  }
  if (Object.getPrototypeOf(value) !== Array.prototype) {
    return invalidInput(path, "must be a non-Proxy plain array");
  }

  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Reflect.ownKeys(descriptors);
  if (keys.some((key) => typeof key === "symbol")) {
    return invalidInput(path, "must not contain symbol keys");
  }
  const lengthDescriptor = Reflect.get(descriptors, "length") as PropertyDescriptor | undefined;
  if (lengthDescriptor === undefined || !("value" in lengthDescriptor)) {
    return invalidInput(`${path}.length`, "must be a data property");
  }
  const length = lengthDescriptor.value as number;
  const allowed = new Set<string>(["length"]);
  const snapshot: string[] = [];
  const seen = new Set<string>();
  for (let index = 0; index < length; index += 1) {
    const name = String(index);
    allowed.add(name);
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || descriptor.enumerable !== true) {
      return invalidInput(`${path}[${index}]`, "must be an enumerable data property");
    }
    const entry = descriptor.value;
    if (typeof entry !== "string" || entry.length === 0 || entry !== entry.trim()) {
      return invalidInput(`${path}[${index}]`, "must be a canonical non-empty string");
    }
    if (seen.has(entry)) return invalidInput(path, "must not contain duplicates");
    seen.add(entry);
    snapshot.push(entry);
  }
  const extra = keys.find((key) => typeof key === "string" && !allowed.has(key));
  if (extra !== undefined) return invalidInput(`${path}.${String(extra)}`, "is not allowed");
  return Object.freeze(snapshot);
};

type ThenHandler<Value> = (
  onFulfilled: (value: Value) => void,
  onRejected: (reason: unknown) => void,
) => unknown;

const captureThen = <Value>(value: Value | PromiseLike<Value>): ThenHandler<Value> | undefined => {
  if ((typeof value !== "object" && typeof value !== "function") || value === null) {
    return undefined;
  }
  const then = (value as { readonly then?: unknown }).then;
  return typeof then === "function" ? (then as ThenHandler<Value>).bind(value) : undefined;
};

/**
 * Validate the complete request before ownership mutation or service
 * construction. The only constructible release path is local and non-model.
 */
export function constructMinimalService<Service>(
  input: MinimalServiceInput<Promise<Service>>,
): Promise<ConstructedMinimalService<Service>>;
export function constructMinimalService<Service>(
  input: MinimalServiceInput<Service>,
): ConstructedMinimalService<Service>;
export function constructMinimalService<Service>(
  input: MinimalServiceInput<Service | Promise<Service>>,
): ConstructedMinimalService<Service> | Promise<ConstructedMinimalService<Service>>;
export function constructMinimalService<Service>(
  input: MinimalServiceInput<Service | Promise<Service>>,
): ConstructedMinimalService<Service> | Promise<ConstructedMinimalService<Service>> {
  const inputSnapshot = snapshotRecord(input, "input", [
    "request",
    "ownership",
    "constructLocalNonModelService",
  ]);
  const requestSnapshot = snapshotRecord(
    inputSnapshot.request,
    "input.request",
    ["authority", "rootIdentity", "profileId"],
    ["requestedCapabilities", "enabledRuntimeDriverIds"],
  );
  const requestedCapabilitiesValue = requestSnapshot.requestedCapabilities;
  const enabledRuntimeDriverIdsValue = requestSnapshot.enabledRuntimeDriverIds;
  const requestedCapabilities =
    requestedCapabilitiesValue === undefined
      ? undefined
      : snapshotStringArray(requestedCapabilitiesValue, "input.request.requestedCapabilities");
  const enabledRuntimeDriverIds =
    enabledRuntimeDriverIdsValue === undefined
      ? undefined
      : snapshotStringArray(enabledRuntimeDriverIdsValue, "input.request.enabledRuntimeDriverIds");
  const request = Object.freeze({
    authority: requestSnapshot.authority as MinimalServiceConstructionRequest["authority"],
    rootIdentity: requestSnapshot.rootIdentity as string,
    profileId: requestSnapshot.profileId as string,
    ...(requestedCapabilities === undefined
      ? {}
      : { requestedCapabilities }),
    ...(enabledRuntimeDriverIds === undefined
      ? {}
      : { enabledRuntimeDriverIds }),
  });
  validateMinimalReleaseRequest({
    requestedCapabilities: request.requestedCapabilities ?? [LOCAL_NON_MODEL_CAPABILITY],
    ...(request.enabledRuntimeDriverIds === undefined
      ? {}
      : { enabledRuntimeDriverIds: request.enabledRuntimeDriverIds }),
  });

  const ownershipProvider = inputSnapshot.ownership as RootProfileOwnership;
  const constructLocalNonModelService = inputSnapshot.constructLocalNonModelService as () =>
    | Service
    | Promise<Service>;
  const ownership = ownershipProvider.acquire(request);
  try {
    const service = constructLocalNonModelService();
    const then = captureThen(service);
    if (then !== undefined) {
      const settledService = new Promise<Service>((resolve, reject) => {
        try {
          then(resolve, reject);
        } catch (error: unknown) {
          reject(error);
        }
      });
      return settledService.then(
        (resolved) => ({ service: resolved, ownership }),
        (error: unknown) => {
          ownership.release();
          throw error;
        },
      );
    }
    return {
      service: service as Service,
      ownership,
    };
  } catch (error: unknown) {
    ownership.release();
    throw error;
  }
}
