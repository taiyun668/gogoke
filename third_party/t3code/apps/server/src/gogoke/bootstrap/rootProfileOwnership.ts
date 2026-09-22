export type ServiceAuthority = "legacy" | "public";

export interface RootProfileIdentity {
  readonly rootIdentity: string;
  readonly profileId: string;
}

export interface RootProfileOwnershipRequest extends RootProfileIdentity {
  readonly authority: ServiceAuthority;
}

export interface RootProfileOwnershipLease extends RootProfileOwnershipRequest {
  readonly release: () => void;
}

export class RootProfileOwnershipError extends Error {
  readonly requestedAuthority: ServiceAuthority;
  readonly existingAuthority: ServiceAuthority;

  constructor(input: {
    readonly requestedAuthority: ServiceAuthority;
    readonly existingAuthority: ServiceAuthority;
  }) {
    super(
      `ROOT_PROFILE_ALREADY_OWNED: ${input.existingAuthority} already owns the requested root/profile; refusing ${input.requestedAuthority}`,
    );
    this.name = "RootProfileOwnershipError";
    this.requestedAuthority = input.requestedAuthority;
    this.existingAuthority = input.existingAuthority;
  }
}

const assertOpaqueIdentity: (label: string, value: unknown) => asserts value is string = (
  label,
  value,
) => {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    throw new Error(`INVALID_ROOT_PROFILE_IDENTITY: ${label} must be non-empty and canonical`);
  }
};

const ownershipKey = (identity: RootProfileIdentity): string =>
  JSON.stringify([identity.rootIdentity, identity.profileId]);

type OwnerRecord = { readonly authority: ServiceAuthority; readonly token: symbol };

const OWNER_REGISTRY_KEY = Symbol.for("gogoke.root-profile-ownership.v1");
type GlobalWithOwnerRegistry = typeof globalThis & {
  readonly [OWNER_REGISTRY_KEY]?: Map<string, OwnerRecord>;
};

const globalRegistry = globalThis as GlobalWithOwnerRegistry;
if (globalRegistry[OWNER_REGISTRY_KEY] === undefined) {
  Object.defineProperty(globalRegistry, OWNER_REGISTRY_KEY, {
    value: new Map<string, OwnerRecord>(),
    enumerable: false,
    configurable: false,
    writable: false,
  });
}
const owners = globalRegistry[OWNER_REGISTRY_KEY]!;

/**
 * Process-local construction fence. The OS root lock supplied by the native
 * host remains the cross-process authority.
 */
export class RootProfileOwnership {
  acquire(request: RootProfileOwnershipRequest): RootProfileOwnershipLease {
    const authority = request.authority;
    const rootIdentity = request.rootIdentity;
    const profileId = request.profileId;
    if (authority !== "legacy" && authority !== "public") {
      throw new Error("INVALID_ROOT_PROFILE_IDENTITY: authority must be legacy or public");
    }
    assertOpaqueIdentity("rootIdentity", rootIdentity);
    assertOpaqueIdentity("profileId", profileId);
    const snapshot = Object.freeze({ authority, rootIdentity, profileId });

    const key = ownershipKey(snapshot);
    const existing = owners.get(key);
    if (existing !== undefined) {
      throw new RootProfileOwnershipError({
        requestedAuthority: snapshot.authority,
        existingAuthority: existing.authority,
      });
    }

    const token = Symbol("root-profile-owner");
    owners.set(key, { authority: snapshot.authority, token });
    let released = false;

    return {
      ...snapshot,
      release: () => {
        if (released) return;
        released = true;
        const current = owners.get(key);
        if (current?.token === token) {
          owners.delete(key);
        }
      },
    };
  }
}
