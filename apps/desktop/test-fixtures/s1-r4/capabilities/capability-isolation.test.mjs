import assert from "node:assert/strict";
import test from "node:test";

import { CapabilityCatalogService, qualificationKey } from "../../../../../third_party/t3code/apps/server/src/gogoke/capabilities/catalog.ts";
import { parseAdapterManifest } from "../../../../../third_party/t3code/apps/server/src/gogoke/runtimeCatalog/manifest.ts";

const digest = (character) => `sha256:${character.repeat(64)}`;

function authority(instanceId, authRevision = "7") {
  const driverId = "fixture_driver";
  const manifest = parseAdapterManifest({
    schema: "gogoke.adapter-manifest.v1",
    packageId: "fixture.capability-driver",
    driverId,
    adapterVersion: "1.0.0",
    artifactDigest: digest("a"),
    hostApiRange: { minInclusive: "1.0.0", maxExclusive: "2.0.0" },
    nativeProtocol: "fixture-jsonl",
    platforms: ["win32-x64"],
    configSchemaRef: "schema://fixture/v1",
    declaredCapabilities: [{ name: "prompt", support: "supported" }],
    requiredHostServices: ["process.spawn"],
    requestedEffects: ["process.spawn"],
    source: { kind: "bundled", provenanceRef: "fixture://driver" },
    license: { spdxId: "MIT", noticeRef: "fixture://notice" },
    admissionRef: "fixture://admission",
  });
  const target = {
    driver: {
      driver: {
        driverId,
        adapterVersion: manifest.adapterVersion,
        artifactDigest: manifest.artifactDigest,
        configSchemaRef: manifest.configSchemaRef,
        requiredHostServices: manifest.requiredHostServices,
        admissionRef: manifest.admissionRef,
      },
      manifest,
    },
    instance: {
      instance: {
        instanceId,
        driverId,
        binaryIdentity: digest("b"),
        profileRef: "profile://shared",
        authRevision,
        hostRef: "host://local",
        capacityPoolRef: "capacity://shared",
      },
      adapterVersion: manifest.adapterVersion,
      accountRef: "account_shared",
      enabled: true,
      config: {},
    },
    model: {
      runtimeInstanceId: instanceId,
      nativeModelId: "model-shared",
      resolvedVersion: "2026-09-20",
      capabilityRevision: "3",
    },
    seat: {
      seatId: `seat_${instanceId}`,
      roleId: "role_fixture",
      scope: "fixture",
      domainId: "domain-fixture",
      lifecycle: "active",
      grantRef: "grant://seat",
    },
  };
  return Object.freeze({
    target,
    readyHost: Object.freeze({
      authority: "public",
      binding: Object.freeze({
        rootIdentity: "root://fixture",
        profileId: "profile://shared",
        runtimeInstanceId: instanceId,
        authRevision,
        generation: "11",
      }),
      sourceEpoch: "13",
      custodyRef: "custody://fixture",
    }),
    environment: Object.freeze({
      binaryDigest: digest("b"),
      nativeDigest: digest("c"),
      platform: "win32-x64",
      runtimeMode: "managed",
      isolationProfile: "workspace-write",
      toolProfile: "bounded-tools",
    }),
  });
}

function fixture(initial) {
  const readyHost = {
    currentValue: initial,
    current(subject) {
      return subject.runtimeInstanceId === this.currentValue.target.instance.instance.instanceId
        ? this.currentValue
        : null;
    },
    transition() {
      return null;
    },
  };
  const service = new CapabilityCatalogService(
    {
      readyHost,
      grants: {
        resolvePassiveGrant: (_subject, grantRef) =>
          grantRef === "grant://L2"
            ? { level: "L2", grantRef, grantRevision: "1", mode: "passive-only" }
            : null,
      },
      sources: {
        resolveSource: (_subject, sourceRef, kind) =>
          sourceRef.startsWith("fixture://")
            ? { kind, ref: sourceRef, revision: "1" }
            : null,
      },
    },
    {
      driverId: "fixture_driver",
      runtimeInstanceId: initial.target.instance.instance.instanceId,
      manifestRevision: "1",
    },
  );
  return { readyHost, service };
}

function qualify(service, support) {
  service.observe({
    observationRevision: "1",
    grantRef: "grant://L2",
    capabilities: [
      { name: "prompt", support, requiredLevel: "L1", sourceRef: "fixture://prompt" },
    ],
    capacity: { status: "unknown", sourceRef: "fixture://capacity" },
  });
  service.qualify({
    observationRevision: "1",
    grantRef: "grant://L2",
    sourceRef: "fixture://qualification",
    validUntilEpochMs: 2_000,
  });
}

test("same account and model keep runtime instances and qualifications separate", () => {
  const first = fixture(authority("instance_alpha")).service;
  const second = fixture(authority("instance_beta")).service;
  qualify(first, "supported");
  qualify(second, "unsupported");
  assert.notEqual(
    qualificationKey(first.view().currentIdentity),
    qualificationKey(second.view().currentIdentity),
  );
  assert.equal(first.resolve("prompt", 1_000).support, "supported");
  assert.equal(second.resolve("prompt", 1_000).support, "unsupported");
});

test("unknown capacity remains unknown and never acquires a numeric zero", () => {
  const { service } = fixture(authority("instance_alpha"));
  qualify(service, "supported");
  const capacity = service.capacity(1_000);
  assert.equal(capacity.status, "unknown");
  assert.equal("available" in capacity, false);
});

test("auth revision drift invalidates new work without rewriting the old binding", () => {
  const { readyHost, service } = fixture(authority("instance_alpha"));
  qualify(service, "supported");
  const binding = service.begin("binding-one", ["prompt"], 1_000);
  readyHost.currentValue = authority("instance_alpha", "8");
  assert.equal(service.resolve("prompt", 1_000).reason, "HOST_TRANSITION_REQUIRED");
  assert.throws(() => service.begin("binding-two", ["prompt"], 1_000));
  assert.equal(service.snapshotBinding(binding), binding);
  assert.equal(Object.isFrozen(binding), true);
});
