import * as NodeAssert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import type {
  ModelRef,
  RoleId,
  RuntimeAccountRef,
  RuntimeDriverId,
  RuntimeInstanceId,
  SeatId,
} from "../contracts/model.ts";
import type { ReadyHost } from "../host/coordinator.ts";
import { parseAdapterManifest } from "../runtimeCatalog/manifest.ts";
import { ADAPTER_MANIFEST_SCHEMA, type RuntimeExecutionTarget } from "../runtimeCatalog/types.ts";
import { CapabilityCatalogService, qualificationKey } from "./catalog.ts";
import {
  CapabilityCatalogError,
  type CapabilityAuthorityAdapters,
  type CapabilitySourceKind,
  type CapabilitySubjectRef,
  type HostTransitionEvidence,
  type PassiveObservationRequest,
  type QualificationIdentity,
  type RuntimeAuthoritySnapshot,
} from "./types.ts";

const assert: typeof NodeAssert = NodeAssert;
const digest = (character: string): string => `sha256:${character.repeat(64)}`;

const target = (
  options: {
    readonly instanceId?: string;
    readonly profileRef?: string;
    readonly accountRef?: string | null;
    readonly authRevision?: string;
    readonly modelVersion?: string;
  } = {},
): RuntimeExecutionTarget => {
  const driverId = "fixture_driver" as RuntimeDriverId;
  const runtimeInstanceId = (options.instanceId ?? "instance_alpha") as RuntimeInstanceId;
  const profileRef = options.profileRef ?? "profile://alpha";
  const authRevision = options.authRevision ?? "7";
  const accountRef = options.accountRef === undefined ? "account_alpha" : options.accountRef;
  const manifest = parseAdapterManifest({
    schema: ADAPTER_MANIFEST_SCHEMA,
    packageId: "local.fixture-driver",
    driverId,
    adapterVersion: "1.2.3",
    artifactDigest: digest("a"),
    hostApiRange: { minInclusive: "1.0.0", maxExclusive: "2.0.0" },
    nativeProtocol: "fixture-jsonl",
    platforms: ["win32-x64"],
    configSchemaRef: "schema://fixture/v1",
    declaredCapabilities: [
      { name: "prompt", support: "supported" },
      { name: "interrupt", support: "unsupported" },
      { name: "usage", support: "unknown" },
    ],
    requiredHostServices: ["process.spawn"],
    requestedEffects: ["process.spawn"],
    source: { kind: "bundled", provenanceRef: "source://fixture-driver" },
    license: { spdxId: "MIT", noticeRef: "notice://fixture-driver" },
    admissionRef: "admission://fixture-driver",
  });
  return {
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
        instanceId: runtimeInstanceId,
        driverId,
        binaryIdentity: digest("b"),
        profileRef,
        authRevision:
          authRevision as RuntimeExecutionTarget["instance"]["instance"]["authRevision"],
        hostRef: "host://local",
        capacityPoolRef: "capacity://alpha",
      },
      adapterVersion: manifest.adapterVersion,
      ...(accountRef === null ? {} : { accountRef: accountRef as RuntimeAccountRef }),
      enabled: true,
      config: {},
    },
    model: {
      runtimeInstanceId,
      nativeModelId: "model-alpha" as ModelRef["nativeModelId"],
      resolvedVersion: options.modelVersion ?? "2026-09-20",
      capabilityRevision: "3" as ModelRef["capabilityRevision"],
    },
    seat: {
      seatId: "seat_alpha" as SeatId,
      roleId: "role_alpha" as RoleId,
      scope: "test",
      domainId: "domain-alpha",
      lifecycle: "active",
      grantRef: "grant://seat-alpha",
    },
  };
};

const authority = (
  options: {
    readonly sourceEpoch?: string;
    readonly generation?: string;
    readonly instanceId?: string;
    readonly profileRef?: string;
    readonly accountRef?: string | null;
    readonly authRevision?: string;
    readonly modelVersion?: string;
    readonly binaryDigest?: string;
    readonly nativeDigest?: string;
    readonly platform?: string;
    readonly runtimeMode?: string;
    readonly isolationProfile?: string;
    readonly toolProfile?: string;
  } = {},
): RuntimeAuthoritySnapshot => {
  const selectedTarget = target(options);
  const instance = selectedTarget.instance.instance;
  const readyHost: ReadyHost = Object.freeze({
    authority: "public",
    binding: Object.freeze({
      rootIdentity: "root://fixture",
      profileId: instance.profileRef,
      runtimeInstanceId: instance.instanceId,
      authRevision: instance.authRevision,
      generation: options.generation ?? "11",
    }),
    sourceEpoch: options.sourceEpoch ?? "13",
    custodyRef: "custody://fixture",
  });
  return Object.freeze({
    target: selectedTarget,
    readyHost,
    environment: Object.freeze({
      binaryDigest: options.binaryDigest ?? digest("b"),
      nativeDigest: options.nativeDigest ?? digest("c"),
      platform: options.platform ?? "win32-x64",
      runtimeMode: options.runtimeMode ?? "managed",
      isolationProfile: options.isolationProfile ?? "workspace-write",
      toolProfile: options.toolProfile ?? "bounded-tools",
    }),
  });
};

class FakeReadyHostAuthority {
  currentValue: RuntimeAuthoritySnapshot;
  readonly transitions = new Map<string, HostTransitionEvidence>();

  constructor(initial: RuntimeAuthoritySnapshot) {
    this.currentValue = initial;
  }

  current(subject: CapabilitySubjectRef): RuntimeAuthoritySnapshot | null {
    const instance = this.currentValue.target.instance.instance;
    return subject.driverId === instance.driverId &&
      subject.runtimeInstanceId === instance.instanceId
      ? this.currentValue
      : null;
  }

  transition(_subject: CapabilitySubjectRef, evidenceRef: string): HostTransitionEvidence | null {
    const evidence = this.transitions.get(evidenceRef) ?? null;
    if (evidence !== null) this.currentValue = evidence.next;
    return evidence;
  }

  addTransition(evidenceRef: string, next: RuntimeAuthoritySnapshot): void {
    this.transitions.set(
      evidenceRef,
      Object.freeze({ evidenceRef, previous: this.currentValue, next }),
    );
  }
}

const fixture = (initial = authority()) => {
  const readyHost = new FakeReadyHostAuthority(initial);
  const adapters: CapabilityAuthorityAdapters = {
    readyHost,
    grants: {
      resolvePassiveGrant: (_subject, grantRef) => {
        if (grantRef !== "grant://L1" && grantRef !== "grant://L2") return null;
        return Object.freeze({
          level: grantRef.endsWith("L2") ? "L2" : "L1",
          grantRef,
          grantRevision: "2",
          mode: "passive-only",
        });
      },
    },
    sources: {
      resolveSource: (_subject, sourceRef, expectedKind: CapabilitySourceKind) => {
        if (!sourceRef.startsWith("fixture://")) return null;
        return Object.freeze({ kind: expectedKind, ref: sourceRef, revision: "5" });
      },
    },
  };
  const service = new CapabilityCatalogService(adapters, {
    driverId: "fixture_driver" as RuntimeDriverId,
    runtimeInstanceId: initial.target.instance.instance.instanceId,
    manifestRevision: "4",
  });
  return { service, adapters, readyHost };
};

const observation = (
  revision = "5",
  promptSupport: "supported" | "unsupported" | "unknown" = "supported",
): PassiveObservationRequest => ({
  observationRevision: revision,
  grantRef: "grant://L2",
  capabilities: [
    {
      name: "prompt",
      support: promptSupport,
      requiredLevel: "L1",
      sourceRef: "fixture://observation/prompt",
    },
    {
      name: "interrupt",
      support: "unsupported",
      requiredLevel: "L2",
      sourceRef: "fixture://observation/interrupt",
    },
    {
      name: "usage",
      support: "unknown",
      requiredLevel: "L1",
      sourceRef: "fixture://observation/usage",
    },
  ],
  capacity: { status: "unknown", sourceRef: "fixture://observation/capacity" },
});

const qualify = (service: CapabilityCatalogService, revision = "5"): void => {
  service.qualify({
    observationRevision: revision,
    grantRef: "grant://L2",
    sourceRef: "fixture://qualification/batch",
    validUntilEpochMs: 2_000,
  });
};

describe("S1-09A-P trusted capability service", () => {
  it("gogoke-s1-r4/T06.L does not let request callers mint grants, sources, or qualifications", () => {
    const { service } = fixture();
    assert.throws(
      () =>
        service.observe({
          ...observation(),
          grantRef: { level: "L2", mode: "passive-only" } as unknown as string,
        }),
      CapabilityCatalogError,
    );
    assert.throws(
      () =>
        service.observe({
          ...observation(),
          capabilities: [
            {
              ...observation().capabilities[0]!,
              sourceRef: {
                kind: "qualification",
                ref: "forged",
              } as unknown as string,
            },
          ],
        }),
      CapabilityCatalogError,
    );
    assert.equal(service.view().qualified.length, 0);
  });

  it("gogoke-s1-r4/T06.L rejects observation accessors without reading them", () => {
    const { service } = fixture();
    let reads = 0;
    const request = observation();
    Object.defineProperty(request, "observationRevision", {
      enumerable: true,
      get: () => {
        reads += 1;
        return "5";
      },
    });
    assert.throws(() => service.observe(request), CapabilityCatalogError);
    assert.equal(reads, 0);
    assert.equal(service.view().qualified.length, 0);
  });

  it("gogoke-s1-r4/T06.L captures adapters once and rejects service/view clones", () => {
    const { service, adapters } = fixture();
    const capturedCurrent = adapters.readyHost.current;
    adapters.readyHost.current = () => null;
    assert.doesNotThrow(() => service.observe(observation()));
    adapters.readyHost.current = capturedCurrent;

    const clone = { ...service.view() };
    assert.equal("observe" in clone, false);
    assert.throws(
      () => CapabilityCatalogService.prototype.qualify.call(clone, {} as never),
      TypeError,
    );
    assert.equal(service.view().qualified.length, 0);
  });

  it("gogoke-s1-r4/T06.L preserves sourced supported/unsupported/unknown and unknown capacity", () => {
    const { service } = fixture();
    service.observe(observation());
    qualify(service);
    assert.deepEqual(
      [
        service.resolve("prompt", 1_000).support,
        service.resolve("interrupt", 1_000).support,
        service.resolve("usage", 1_000).support,
      ],
      ["supported", "unsupported", "unknown"],
    );
    for (const name of ["prompt", "interrupt", "usage"]) {
      assert.equal(service.resolve(name, 1_000).sources[0]?.kind, "qualification");
    }
    const capacity = service.capacity(1_000);
    assert.equal(capacity.status, "unknown");
    assert.equal("available" in capacity, false);
    assert.equal(capacity.source.kind, "qualification");
  });

  it("gogoke-s1-r4/T06.L keeps declaration and L1 evidence below L2 qualification", () => {
    const { service } = fixture();
    assert.equal(service.resolve("prompt", 1_000).support, "unknown");
    assert.throws(
      () => service.observe({ ...observation(), grantRef: "grant://L1" }),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORIZATION_INSUFFICIENT",
    );
    assert.throws(
      () => service.observe({ ...observation(), grantRef: "missing-grant" }),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORIZATION_REQUIRED",
    );
  });

  it("gogoke-s1-r4/T06.L reads only passive exact observations", () => {
    const { service } = fixture();
    let reads = 0;
    const active = { ...observation().capabilities[0] } as Record<string, unknown>;
    Object.defineProperty(active, "name", {
      enumerable: true,
      get: () => {
        reads += 1;
        return "prompt";
      },
    });
    assert.throws(
      () => service.observe({ ...observation(), capabilities: [active as never] }),
      CapabilityCatalogError,
    );
    assert.equal(reads, 0);
  });

  it("gogoke-s1-r4/T58.L makes identical revision/content idempotent and conflicts on changed digest", () => {
    const { service } = fixture();
    const first = service.observe(observation());
    const second = service.observe(observation());
    assert.equal(first.observations.length, 1);
    assert.equal(second.observations.length, 1);
    assert.equal(
      first.observations[0]?.observationDigest,
      second.observations[0]?.observationDigest,
    );
    assert.throws(
      () => service.observe(observation("5", "unsupported")),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "OBSERVATION_REVISION_CONFLICT",
    );
  });

  it("gogoke-s1-r4/T58.L higher observation retires digest and all bound capability/capacity authority", () => {
    const { service } = fixture();
    service.observe(observation());
    qualify(service);
    const inFlight = service.begin("binding-alpha", ["prompt"], 1_000);
    const oldDigest = inFlight.observationDigest;
    const updated = service.observe(observation("6"));

    assert.equal(updated.retiredObservationDigests.includes(oldDigest), true);
    assert.equal(updated.qualified.length, 0);
    assert.equal(updated.qualifiedCapacity.length, 0);
    assert.equal(service.resolve("prompt", 1_000).reason, "NOT_QUALIFIED");
    assert.equal(service.capacity(1_000).status, "unknown");
    assert.strictEqual(service.snapshotBinding(inFlight), inFlight);
    assert.equal(inFlight.capabilities[0]?.support, "supported");
  });

  it("gogoke-s1-r4/T58.L revoked observation invalidates its exact qualification digest", () => {
    const { service } = fixture();
    const digestValue = service.observe(observation()).observations[0]!.observationDigest;
    qualify(service);
    const view = service.revokeObservation({
      observationDigest: digestValue,
      sourceRef: "fixture://revocation/observation",
    });
    assert.equal(view.qualified.length, 0);
    assert.equal(view.qualifiedCapacity.length, 0);
    assert.equal(service.resolve("prompt", 1_000).support, "unknown");
    assert.throws(
      () => qualify(service),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "OBSERVATION_NOT_FOUND",
    );
  });

  it("gogoke-s1-r4/T58.L requires trusted transition evidence before any new request authority", () => {
    const { service, readyHost } = fixture();
    service.observe(observation());
    qualify(service);
    readyHost.currentValue = authority({ sourceEpoch: "14", generation: "12" });
    assert.equal(service.resolve("prompt", 1_000).reason, "HOST_TRANSITION_REQUIRED");
    assert.throws(
      () => service.begin("binding-beta", ["prompt"], 1_000),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "HOST_TRANSITION_REQUIRED",
    );
  });

  it("gogoke-s1-r4/T58.L rejects generation high-water rollback 11 to 12 to 11", () => {
    const { service, readyHost } = fixture();
    const next = authority({ sourceEpoch: "14", generation: "12", authRevision: "8" });
    readyHost.addTransition("transition://up", next);
    service.transition("transition://up");
    const rollback = authority({ sourceEpoch: "15", generation: "11", authRevision: "9" });
    readyHost.addTransition("transition://rollback", rollback);
    assert.throws(
      () => service.transition("transition://rollback"),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORITY_ROLLBACK",
    );
  });

  it("gogoke-s1-r4/T58.L rejects auth revision rollback and transition replay", () => {
    const { service, readyHost } = fixture();
    readyHost.addTransition(
      "transition://auth-up",
      authority({ sourceEpoch: "14", generation: "12", authRevision: "8" }),
    );
    service.transition("transition://auth-up");
    assert.throws(
      () => service.transition("transition://auth-up"),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORITY_REPLAY",
    );
    readyHost.addTransition(
      "transition://auth-down",
      authority({ sourceEpoch: "15", generation: "13", authRevision: "7" }),
    );
    assert.throws(
      () => service.transition("transition://auth-down"),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORITY_ROLLBACK",
    );
  });

  it("gogoke-s1-r4/T58.L tombstones retired profile and account values", () => {
    const { service, readyHost } = fixture();
    const changed = authority({
      sourceEpoch: "14",
      generation: "12",
      authRevision: "8",
      profileRef: "profile://beta",
      accountRef: "account_beta",
    });
    readyHost.addTransition("transition://change-principal", changed);
    service.transition("transition://change-principal");
    const revive = authority({
      sourceEpoch: "15",
      generation: "13",
      authRevision: "9",
      profileRef: "profile://alpha",
      accountRef: "account_alpha",
    });
    readyHost.addTransition("transition://revive-principal", revive);
    assert.throws(
      () => service.transition("transition://revive-principal"),
      (error: unknown) =>
        error instanceof CapabilityCatalogError && error.code === "AUTHORITY_ROLLBACK",
    );
  });

  it("gogoke-s1-r4/T58.L never revives old qualification when non-counter fields return", () => {
    const { service, readyHost } = fixture();
    service.observe(observation());
    qualify(service);
    const firstKey = qualificationKey(service.view().currentIdentity);
    const changed = authority({ sourceEpoch: "14", modelVersion: "2026-09-21" });
    readyHost.addTransition("transition://model-new", changed);
    const afterChange = service.transition("transition://model-new");
    assert.equal(afterChange.retiredQualificationKeys.includes(firstKey), true);
    const revived = authority({ sourceEpoch: "15", modelVersion: "2026-09-20" });
    readyHost.addTransition("transition://model-old", revived);
    service.transition("transition://model-old");
    assert.notEqual(qualificationKey(service.view().currentIdentity), firstKey);
    assert.equal(service.resolve("prompt", 1_000).reason, "NOT_QUALIFIED");
  });

  it("gogoke-s1-r4/T58.L keeps every qualification dimension in the provider-neutral key", () => {
    const base = fixture().service.view().currentIdentity;
    const changes: ReadonlyArray<Partial<QualificationIdentity>> = [
      { driverId: "other_driver" as RuntimeDriverId },
      { binaryDigest: digest("d") },
      { adapterDigest: digest("e") },
      { nativeDigest: digest("f") },
      { platform: "win32-arm64" },
      { profileRef: "profile://beta" },
      { accountRef: "account_beta" },
      { authRevision: "8" },
      { runtimeInstanceId: "instance_beta" as RuntimeInstanceId },
      { nativeModelId: "model-beta" },
      { resolvedModelVersion: "2026-09-21" },
      { runtimeMode: "stdio" },
      { isolationProfile: "read-only" },
      { toolProfile: "no-tools" },
      { generation: "12" },
      { sourceEpoch: "14" },
    ];
    for (const change of changes) {
      assert.notEqual(qualificationKey(base), qualificationKey({ ...base, ...change }));
    }
  });

  it("gogoke-s1-r4/T58.L keeps instances separate even under one account/model", () => {
    const first = fixture(authority({ accountRef: "account_shared" })).service;
    const second = fixture(
      authority({ instanceId: "instance_beta", accountRef: "account_shared" }),
    ).service;
    assert.notEqual(
      qualificationKey(first.view().currentIdentity),
      qualificationKey(second.view().currentIdentity),
    );
  });

  it("gogoke-s1-r4/T58.L freezes authentic in-flight bindings and rejects clones", () => {
    const { service } = fixture();
    service.observe(observation());
    qualify(service);
    const binding = service.begin("binding-alpha", ["prompt"], 1_000);
    assert.equal(Object.isFrozen(binding), true);
    assert.equal(Object.isFrozen(binding.capabilities), true);
    assert.throws(() => service.snapshotBinding({ ...binding }), CapabilityCatalogError);
    assert.strictEqual(service.snapshotBinding(binding), binding);
  });
});
