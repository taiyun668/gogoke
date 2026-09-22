import type { ContextManifest, ContextObject, ContextState } from "../../contracts/model.ts";

/** Context assembly is preparatory until these ports are bound to Product Authority. */
export interface AssemblyIdentity {
  readonly principalId: string;
  readonly seatId: string;
  readonly taskId: string;
  readonly sessionId: string;
  readonly domainId: string;
  readonly bindingId: string;
  readonly bindingGeneration: string;
  readonly sourceEpoch: string;
  readonly runtimeInstanceId: string;
}

export interface AssemblyRequest extends AssemblyIdentity {
  readonly operationId: string;
  readonly manifestId: string;
  readonly query: string;
  /** Caller may reduce, never raise, the authoritative content-byte ceiling. */
  readonly maxContentBytes: number;
}

export interface ContextVersionRef {
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
}

/** Resolved by the existing grant authority, not a grant supplied by the caller. */
export interface ReadPartition {
  readonly sourceDomainId: string;
  readonly authorizationRef: string;
  readonly authorizationRevision: string;
}

export interface AssemblyBasis extends AssemblyIdentity {
  /** Opaque reference only. Every port must resolve it again, not trust its syntax. */
  readonly readRef: string;
  /** Existing ActionAdmission identity; never issued by this module. */
  readonly admissionRef: string;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly authRevision: string;
  readonly revocationHead: string;
  readonly selectionDecisionId: string;
  readonly maxContentBytes: number;
  readonly maxCandidates: number;
  readonly partitions: ReadonlyArray<ReadPartition>;
  readonly requiredConstraints: ReadonlyArray<ContextVersionRef>;
}

/** Current status is separate from the immutable ContextObject's original fields. */
export interface ResolvedContextVersion {
  readonly object: ContextObject;
  readonly content: string;
  readonly state: ContextState;
  readonly stateRevision: string;
  readonly accessPolicyRevision: string;
}

export interface ExpectedContextVersion extends ContextVersionRef {
  readonly contentHash: string;
  readonly stateRevision: string;
  readonly accessPolicyRevision: string;
}

export interface CommitManifestRequest {
  readonly operationId: string;
  readonly requestDigest: string;
  readonly basis: AssemblyBasis;
  readonly expectedVersions: ReadonlyArray<ExpectedContextVersion>;
  readonly manifest: ContextManifest;
}

export type CommitManifestResult =
  | { readonly kind: "committed" | "replayed"; readonly operationId: string;
      readonly manifestId: string; readonly manifestHash: string }
  | { readonly kind: "denied" | "stale" | "conflict" };

export interface ReplayManifestRequest extends AssemblyIdentity {
  readonly operationId: string;
}

/**
 * Internal adapter seam to ONE existing T3-derived Product Authority / Route-B store.
 * It is not exported by the public bootstrap and has no default implementation.
 *
 * openAssembly authorizes before any search. searchVisibleContext partitions FTS
 * BEFORE retrieval and returns no inaccessible IDs/counts. All reads resolve the
 * readRef against current principal/seat/binding/grants and revocation state.
 *
 * commitManifest compares task/policy/auth/revocation/binding and every INCLUDED
 * source identity/hash/state/access revision inside the existing authority's
 * transaction and ActionAdmission. It must not hold that transaction over model
 * generation. An unrelated UI revision is not one of these preconditions.
 *
 * readCurrentManifest reauthorizes current access AND binding legitimacy AND
 * source state even on durable replay; null covers absent or inaccessible data.
 * Fake ports test choreography only, not grant correctness, FTS, SQL or acceptance.
 */
export interface ContextAssemblyAuthorityPort {
  openAssembly(request: AssemblyRequest): Promise<AssemblyBasis | null>;
  searchVisibleContext(basis: AssemblyBasis, query: string): Promise<ReadonlyArray<ContextVersionRef>>;
  loadVisibleVersions(basis: AssemblyBasis, references: ReadonlyArray<ContextVersionRef>):
    Promise<ReadonlyArray<ResolvedContextVersion>>;
  commitManifest(request: CommitManifestRequest): Promise<CommitManifestResult>;
  readCurrentManifest(request: ReplayManifestRequest): Promise<ContextManifest | null>;
}

export type AssemblyErrorCode =
  | "INVALID_INPUT" | "AUTHORITY_PROTOCOL_ERROR" | "ACCESS_DENIED"
  | "NEEDS_EVIDENCE" | "NEEDS_BUDGET" | "STALE_ASSEMBLY" | "OPERATION_CONFLICT"
  | "AUTHORITY_UNAVAILABLE" | "COMMIT_OUTCOME_UNKNOWN";

export class ContextAssemblyError extends Error {
  override readonly name = "ContextAssemblyError";
  readonly code: AssemblyErrorCode;
  constructor(code: AssemblyErrorCode) {
    // No inaccessible Context IDs, content, grants or underlying provider errors.
    super(code);
    this.code = code;
  }
}
