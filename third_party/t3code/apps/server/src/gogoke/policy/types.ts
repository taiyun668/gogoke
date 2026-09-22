export const POLICY_ACTIONS = [
  "delegate",
  "return-result",
  "request-review",
  "share-material",
  "answer-continuation",
  "cancel-continuation",
] as const;
export type PolicyAction = (typeof POLICY_ACTIONS)[number];

export const MATERIAL_SINKS = [
  "task-package",
  "stdin",
  "rules",
  "files",
  "log",
  "public-stream",
  "controller",
  "notification",
  "export",
  "cache",
  "restore",
  "formal-review",
] as const;
export type MaterialSink = (typeof MATERIAL_SINKS)[number];

export type PrincipalRole = "controller" | "worker" | "auditor";
export type MaterialVisibility = "project" | "private";
export type DelegationRoute = "controller-worker" | "worker-controller" | "controller-clean-review";

export interface PolicyPrincipal {
  readonly principalId: string;
  readonly projectId: string;
  readonly domainId: string;
  readonly role: PrincipalRole;
}

export interface PolicyBinding {
  readonly sessionId: string;
  readonly executionId: string;
  readonly generation: string;
}

export interface AuthorityGrantParent {
  readonly grantRef: string;
  readonly revision: string;
}

export interface AuthorityCeiling {
  readonly allowedActions: ReadonlyArray<PolicyAction>;
  readonly allowedTargetPrincipalIds: ReadonlyArray<string>;
  readonly allowedTargetDomainIds: ReadonlyArray<string>;
  readonly allowedSinks: ReadonlyArray<MaterialSink>;
  readonly allowedMaterialClasses: ReadonlyArray<string>;
  readonly explicitPrivateMaterialIds: ReadonlyArray<string>;
  readonly allowedContinuationResponses: ReadonlyArray<string>;
  readonly maxMaterialItems: number;
  readonly maxMaterialBytes: number;
  readonly maxResponseBytes: number;
}

/** A grant is resolved by the captured authority port, never supplied inline by a caller. */
export interface AuthorityGrant {
  readonly grantRef: string;
  readonly revision: string;
  readonly revocationHead: string;
  readonly policyRevision: string;
  readonly seatId: string;
  readonly issuerId: string;
  readonly parentGrant: AuthorityGrantParent | null;
  readonly principal: PolicyPrincipal;
  readonly binding: PolicyBinding;
  readonly expiresAtEpochMs: number;
  readonly ceiling: AuthorityCeiling;
}

export interface TaskMaterial {
  readonly materialId: string;
  readonly projectId: string;
  readonly domainId: string;
  readonly ownerPrincipalId: string;
  readonly materialClass: string;
  readonly visibility: MaterialVisibility;
  readonly content: string;
}

export interface ContinuationCeiling {
  readonly allowedResponses: ReadonlyArray<string>;
  readonly maxResponseBytes: number;
}

/** Native request identity supplied by the captured continuation authority port. */
export interface TrustedContinuation {
  readonly continuationId: string;
  readonly nativeRequestId: string;
  readonly requestedAction: string;
  readonly principal: PolicyPrincipal;
  readonly binding: PolicyBinding;
  readonly expiresAtEpochMs: number;
  readonly ceiling: ContinuationCeiling;
}

export interface PolicyAuthorityPort {
  /** Production resolvers read through typed Product Authority operations; in-memory maps are fixtures only. */
  resolveGrant(grantRef: string): Promise<AuthorityGrant | null>;
}

export interface ContinuationAuthorityPort {
  resolveContinuation(nativeRequestId: string): TrustedContinuation | null;
}

export interface MaterialAuthorityPort {
  resolveMaterial(materialId: string): TaskMaterial | null;
}

export interface PolicyAdapters {
  readonly authority: PolicyAuthorityPort;
  readonly continuations: ContinuationAuthorityPort;
  readonly materials: MaterialAuthorityPort;
}

export interface DelegationRequest {
  readonly grantRef: string;
  readonly action: "delegate" | "return-result" | "request-review";
  readonly route: DelegationRoute;
  readonly source: PolicyPrincipal;
  readonly target: PolicyPrincipal;
  readonly sourceBinding: PolicyBinding;
  readonly targetBinding: PolicyBinding;
  readonly targetBindingKind: "existing" | "new-clean";
  readonly sink: "task-package" | "formal-review";
  readonly selectedMaterialIds: ReadonlyArray<string>;
  /** Requested child authority. It must be a structural subset of the resolved parent grant. */
  readonly childCeiling: AuthorityCeiling;
  /** Opaque task body. It is carried after authorization and never parsed for authority. */
  readonly instruction: string;
}

export interface MaterialExposureRequest {
  readonly grantRef: string;
  readonly source: PolicyPrincipal;
  readonly sourceBinding: PolicyBinding;
  readonly targetPrincipalId: string;
  readonly targetDomainId: string;
  readonly sink: MaterialSink;
  readonly selectedMaterialIds: ReadonlyArray<string>;
}

export interface SelectedMaterial {
  readonly materialId: string;
  readonly materialClass: string;
  readonly visibility: MaterialVisibility;
  readonly content: string;
  readonly contentDigest: string;
}

export interface AuthorizedTaskPackage {
  readonly packageDigest: string;
  readonly parentGrantRef: string;
  readonly parentGrantRevision: string;
  readonly parentGrantRevocationHead: string;
  readonly parentPolicyRevision: string;
  readonly parentSeatId: string;
  readonly parentGrantDigest: string;
  readonly parentCeilingDigest: string;
  readonly childCeiling: AuthorityCeiling;
  readonly childCeilingDigest: string;
  readonly action: "delegate" | "return-result" | "request-review";
  readonly route: DelegationRoute;
  readonly source: PolicyPrincipal;
  readonly target: PolicyPrincipal;
  readonly sourceBinding: PolicyBinding;
  readonly targetBinding: PolicyBinding;
  readonly targetBindingKind: "existing" | "new-clean";
  readonly sink: "task-package" | "formal-review";
  readonly instruction: string;
  readonly instructionDigest: string;
  readonly materialSetDigest: string;
  readonly materials: ReadonlyArray<SelectedMaterial>;
}

export interface RevalidatedTaskPackage {
  readonly package: AuthorizedTaskPackage;
  readonly effectiveCeiling: AuthorityCeiling;
  readonly revalidatedAtEpochMs: number;
}

export interface AuthorizedMaterialExposure {
  readonly grantRef: string;
  readonly grantRevision: string;
  readonly sourcePrincipalId: string;
  readonly targetPrincipalId: string;
  readonly sourceDomainId: string;
  readonly targetDomainId: string;
  readonly sink: MaterialSink;
  readonly materials: ReadonlyArray<SelectedMaterial>;
}

export interface ContinuationCommandBase {
  readonly grantRef: string;
  readonly operationId: string;
  readonly continuationId: string;
  readonly nativeRequestId: string;
  readonly requestedAction: string;
  readonly principal: PolicyPrincipal;
  readonly binding: PolicyBinding;
}

export interface AnswerContinuationCommand extends ContinuationCommandBase {
  readonly kind: "answer";
  readonly responseKind: string;
  readonly content: string;
}

export interface CancelContinuationCommand extends ContinuationCommandBase {
  readonly kind: "cancel";
}

export type ContinuationCommand = AnswerContinuationCommand | CancelContinuationCommand;
export type ContinuationState = "pending" | "answered" | "cancelled" | "expired";

export interface ContinuationReceipt {
  readonly kind: "continuation-commit";
  readonly operationId: string;
  readonly continuationId: string;
  readonly nativeRequestId: string;
  readonly requestedAction: string;
  readonly principalId: string;
  readonly domainId: string;
  readonly bindingGeneration: string;
  readonly state: "answered" | "cancelled";
  readonly responseKind: string | null;
  readonly content: string | null;
  readonly committedAtEpochMs: number;
}

export interface ContinuationStopReceipt {
  readonly kind: "stop-pending-verification";
  readonly operationId: string;
  readonly continuationId: string;
  readonly nativeRequestId: string;
  readonly originalCommitOperationId: string;
  readonly principalId: string;
  readonly domainId: string;
  readonly bindingGeneration: string;
  readonly state: "pending-verification";
  readonly requestedAtEpochMs: number;
}

export type ContinuationResolution = ContinuationReceipt | ContinuationStopReceipt;

export interface PendingContinuationQuery {
  readonly principal: PolicyPrincipal;
  readonly binding: PolicyBinding;
}

export type PolicyErrorCode =
  | "INVALID_INPUT"
  | "AUTHORITY_REQUIRED"
  | "AUTHORITY_MISMATCH"
  | "AUTHORITY_EXPIRED"
  | "ACTION_NOT_ALLOWED"
  | "TARGET_NOT_ALLOWED"
  | "SINK_NOT_ALLOWED"
  | "MATERIAL_NOT_FOUND"
  | "MATERIAL_NOT_ALLOWED"
  | "MATERIAL_CEILING_EXCEEDED"
  | "PRIVATE_MATERIAL_NOT_EXPLICIT"
  | "CLEAN_REVIEW_REQUIRED"
  | "CONTINUATION_NOT_FOUND"
  | "CONTINUATION_MISMATCH"
  | "CONTINUATION_EXPIRED"
  | "CONTINUATION_TERMINAL"
  | "OPERATION_CONFLICT"
  | "RESPONSE_NOT_ALLOWED"
  | "RESPONSE_CEILING_EXCEEDED";

export class PolicyError extends Error {
  override readonly name = "PolicyError";
  readonly code: PolicyErrorCode;

  constructor(code: PolicyErrorCode, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}
