import * as NodeAssert from "node:assert/strict";
import * as NodeTest from "node:test";

import type { NativeDelegationGrantSnapshot } from "../persistence/base/nativeHostClient.ts";
import { NativePolicyAuthorityPort } from "./nativePolicyAuthority.ts";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

const grantSnapshot: NativeDelegationGrantSnapshot = {
  binding: { executionId: "execution-one", generation: "18446744073709551615", sessionId: "session-one" },
  ceiling: {
    allowedActions: ["delegate"],
    allowedContinuationResponses: ["continue"],
    allowedMaterialClasses: ["task-context"],
    allowedSinks: ["task-package"],
    allowedTargetDomainIds: ["domain-one"],
    allowedTargetPrincipalIds: ["worker-one"],
    explicitPrivateMaterialIds: ["private-one"],
    maxMaterialBytes: "4096",
    maxMaterialItems: "8",
    maxResponseBytes: "2048",
  },
  expiresAtEpochMs: "4102444800000",
  grantRef: "grant-one",
  issuerId: "owner-issuer",
  parentGrant: { grantRef: "grant-parent", revision: "2" },
  policyRevision: "3",
  principal: {
    domainId: "domain-one",
    principalId: "controller-one",
    projectId: "project-one",
    role: "controller",
    seatId: "seat-one",
  },
  revision: "4",
  revocationHead: "5",
  seatId: "seat-one",
};

const asSnapshot = (value: unknown): NativeDelegationGrantSnapshot =>
  value as NativeDelegationGrantSnapshot;

test("native PolicyAuthorityPort resolves only an immutable native snapshot", async () => {
  const source = JSON.parse(JSON.stringify(grantSnapshot)) as Record<string, unknown>;
  const calls: string[] = [];
  const authority = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async (grantRef) => {
      calls.push(grantRef);
      return asSnapshot(source);
    },
  });

  const resolved = await authority.resolveGrant("grant-one");
  assert.deepEqual(calls, ["grant-one"]);
  assert.equal(resolved?.grantRef, "grant-one");
  assert.equal(resolved?.revision, "4");
  assert.equal(resolved?.revocationHead, "5");
  assert.equal(resolved?.policyRevision, "3");
  assert.equal(resolved?.seatId, "seat-one");
  assert.equal(resolved?.binding.generation, "18446744073709551615");
  assert.equal(resolved?.expiresAtEpochMs, 4102444800000);
  assert.equal(resolved?.ceiling.maxMaterialBytes, 4096);
  assert.equal(Object.isFrozen(resolved), true);
  assert.equal(Object.isFrozen(resolved?.parentGrant), true);
  assert.equal(Object.isFrozen(resolved?.principal), true);
  assert.equal(Object.isFrozen(resolved?.binding), true);
  assert.equal(Object.isFrozen(resolved?.ceiling), true);
  assert.equal(Object.isFrozen(resolved?.ceiling.allowedActions), true);

  const mutablePrincipal = source.principal as Record<string, unknown>;
  mutablePrincipal.seatId = "changed-seat";
  const mutableCeiling = source.ceiling as Record<string, unknown>;
  (mutableCeiling.allowedActions as string[]).push("request-review");
  assert.equal(resolved?.principal.principalId, "controller-one");
  assert.deepEqual(resolved?.ceiling.allowedActions, ["delegate"]);
});

test("native PolicyAuthorityPort fails closed for missing, denied, or malformed authority", async () => {
  assert.throws(() => new NativePolicyAuthorityPort({}), /grant read is unavailable/);

  const missing = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () => null as unknown as NativeDelegationGrantSnapshot,
  });
  assert.equal(await missing.resolveGrant("grant-one"), null);

  const denied = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () => {
      throw new Error("AccessDenied");
    },
  });
  await assert.rejects(denied.resolveGrant("grant-one"), /AccessDenied/);

  const malformed = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () =>
      asSnapshot({ ...grantSnapshot, unexpected: "caller-field" }),
  });
  await assert.rejects(malformed.resolveGrant("grant-one"), /missing or extra fields/);

  const wrongIdentity = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () => grantSnapshot,
  });
  await assert.rejects(wrongIdentity.resolveGrant("grant-other"), /does not match request/);

  const invalidRole = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () =>
      asSnapshot({
        ...grantSnapshot,
        principal: { ...grantSnapshot.principal, role: "owner" },
      }),
  });
  await assert.rejects(invalidRole.resolveGrant("grant-one"), /role.*unsupported/);

  const invalidCeiling = new NativePolicyAuthorityPort({
    readCurrentDelegationGrant: async () =>
      asSnapshot({
        ...grantSnapshot,
        ceiling: { ...grantSnapshot.ceiling, allowedActions: ["delegate", "delete-all"] },
      }),
  });
  await assert.rejects(invalidCeiling.resolveGrant("grant-one"), /unsupported value delete-all/);
});
