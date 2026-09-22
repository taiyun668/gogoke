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
  const request = Object.freeze({
    authority: input.request.authority,
    rootIdentity: input.request.rootIdentity,
    profileId: input.request.profileId,
    ...(input.request.requestedCapabilities === undefined
      ? {}
      : { requestedCapabilities: Object.freeze([...input.request.requestedCapabilities]) }),
    ...(input.request.enabledRuntimeDriverIds === undefined
      ? {}
      : { enabledRuntimeDriverIds: Object.freeze([...input.request.enabledRuntimeDriverIds]) }),
  });
  validateMinimalReleaseRequest({
    requestedCapabilities: request.requestedCapabilities ?? [LOCAL_NON_MODEL_CAPABILITY],
    ...(request.enabledRuntimeDriverIds === undefined
      ? {}
      : { enabledRuntimeDriverIds: request.enabledRuntimeDriverIds }),
  });

  const ownership = input.ownership.acquire(request);
  try {
    const service = input.constructLocalNonModelService();
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
