export const RELEASE_FEATURE_SEALED_CODE = "FEATURE_SEALED";

export const LOCAL_NON_MODEL_CAPABILITY = "local-non-model" as const;

export const SEALED_RELEASE_CAPABILITIES = [
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

export type SealedReleaseCapability = (typeof SEALED_RELEASE_CAPABILITIES)[number];
export type ReleaseCapability = typeof LOCAL_NON_MODEL_CAPABILITY | SealedReleaseCapability;

export const DEFAULT_RUNTIME_DRIVER_IDS: readonly string[] = Object.freeze([]);

export class ReleasePolicyError extends Error {
  readonly capability: string;

  constructor(capability: string, operation: string) {
    super(
      `${RELEASE_FEATURE_SEALED_CODE}: ${capability} is sealed in this release; refusing ${operation}`,
    );
    this.name = "ReleasePolicyError";
    this.capability = capability;
  }
}

/** Validate before resolving or constructing any capability implementation. */
export const assertReleaseCapabilityAllowed = (capability: string, operation: string): void => {
  if (capability !== LOCAL_NON_MODEL_CAPABILITY) {
    throw new ReleasePolicyError(capability, operation);
  }
};

export const validateMinimalReleaseRequest = (input: {
  readonly requestedCapabilities?: readonly string[];
  readonly enabledRuntimeDriverIds?: readonly string[];
}): void => {
  const requestedCapabilities = input.requestedCapabilities ?? [LOCAL_NON_MODEL_CAPABILITY];
  for (const capability of requestedCapabilities) {
    assertReleaseCapabilityAllowed(capability, "minimal service construction");
  }

  const enabledRuntimeDriverIds = input.enabledRuntimeDriverIds ?? DEFAULT_RUNTIME_DRIVER_IDS;
  if (enabledRuntimeDriverIds.length > 0) {
    throw new ReleasePolicyError("model-runtime", "default runtime driver construction");
  }
};

export const constructReleaseCapability = <Service>(
  capability: string,
  operation: string,
  construct: () => Service,
): Service => {
  assertReleaseCapabilityAllowed(capability, operation);
  return construct();
};
