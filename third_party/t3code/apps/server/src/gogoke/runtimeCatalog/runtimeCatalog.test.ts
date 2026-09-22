import * as NodeAssert from "node:assert/strict";
import * as NodeCrypto from "node:crypto";
import { describe, it } from "vite-plus/test";

const assert: typeof NodeAssert = NodeAssert;
const { randomBytes } = NodeCrypto;

import type { ModelRef, RuntimeDriverId, RuntimeInstanceId } from "../contracts/model.ts";
import {
  createCapabilityLayers,
  makeQualificationKey,
  modelIdentityKey,
  resolveQualifiedCapability,
  withObservedCapability,
  withQualifiedCapability,
} from "./capabilities.ts";
import {
  closeBinding,
  decodeRuntimeInstanceConfig,
  encodeRuntimeInstanceConfig,
  openBinding,
  RuntimeCatalog,
} from "./catalog.ts";
import { parseAdapterManifest, RuntimeCatalogError } from "./manifest.ts";
import {
  ADAPTER_MANIFEST_SCHEMA,
  RUNTIME_INSTANCE_CONFIG_SCHEMA,
  type AdapterManifestV1,
  type QualificationKeyInput,
  type ResolvedRuntimeInstanceConfig,
} from "./types.ts";

const digest = (character: string): string => `sha256:${character.repeat(64)}`;

const manifest = (
  driverId: string,
  adapterVersion = "1.0.0",
  artifactDigest = digest("a"),
  hostApiRange = { minInclusive: "1.0.0", maxExclusive: "2.0.0" },
): AdapterManifestV1 =>
  parseAdapterManifest({
    schema: ADAPTER_MANIFEST_SCHEMA,
    packageId: `local.${driverId}`,
    driverId,
    adapterVersion,
    artifactDigest,
    hostApiRange,
    nativeProtocol: "fixture-jsonl",
    platforms: ["win32"],
    configSchemaRef: `schema://${driverId}/config/v1`,
    declaredCapabilities: [
      { name: "prompt", support: "supported" },
      { name: "future.optional", support: "supported" },
    ],
    requiredHostServices: ["process.spawn"],
    requestedEffects: ["process.spawn"],
    source: { kind: "bundled", provenanceRef: `source://${driverId}` },
    license: { spdxId: "MIT", noticeRef: `notice://${driverId}` },
    admissionRef: `admission://${driverId}`,
  });

const catalog = (manifests: ReadonlyArray<AdapterManifestV1>): RuntimeCatalog =>
  new RuntimeCatalog(
    manifests.map((entry) => ({ manifest: entry })),
    {
      hostApiVersion: "1.2.0",
      hostPlatform: "win32",
      availableHostServices: new Set(["process.spawn"]),
      allowedEffects: new Set(["process.spawn"]),
      admittedAdmissionRefs: new Set(manifests.map((entry) => entry.admissionRef)),
    },
  );

const encodedInstance = (overrides: Readonly<Record<string, unknown>> = {}) => ({
  schema: RUNTIME_INSTANCE_CONFIG_SCHEMA,
  instanceId: "workbench",
  driverId: "harness_a",
  adapterVersion: "1.0.0",
  config: { binary: "fixture.exe", nativeExtension: { keep: true } },
  ...overrides,
});

const qualificationInput = (
  overrides: Partial<QualificationKeyInput> = {},
): QualificationKeyInput => ({
  driverId: "harness_a" as RuntimeDriverId,
  adapterDigest: digest("a"),
  nativeDigest: digest("b"),
  platform: "win32-x64",
  profileRef: "profile://one",
  authRevision: "7",
  runtimeInstanceId: "instance_a" as RuntimeInstanceId,
  nativeModelId: "model",
  resolvedModelVersion: "2026-09-01",
  runtimeMode: "managed",
  isolationProfile: "read-only",
  toolProfile: "none",
  generation: "11",
  ...overrides,
});

describe("R4-O-SPI runtime catalog preparatory coverage", () => {
  it("gogoke-s1-r4/R4-01 keeps model identity scoped to the runtime instance", () => {
    const first: ModelRef = {
      runtimeInstanceId: "instance_a" as RuntimeInstanceId,
      nativeModelId: "same-native-model" as ModelRef["nativeModelId"],
      resolvedVersion: "unknown",
      capabilityRevision: "1" as ModelRef["capabilityRevision"],
    };
    const second: ModelRef = {
      ...first,
      runtimeInstanceId: "instance_b" as RuntimeInstanceId,
    };

    NodeAssert.notEqual(modelIdentityKey(first), modelIdentityKey(second));
  });

  it("gogoke-s1-r4/R4-01 round-trips an unknown driver as unavailable", () => {
    const raw = encodedInstance({
      driverId: "unknown_driver",
      adapterVersion: "9.4.1",
      vendorFutureField: { nested: [1, "two", true] },
    });
    const decoded = decodeRuntimeInstanceConfig(raw);
    const resolved = catalog([]).resolveInstance(decoded);

    assert.deepEqual(encodeRuntimeInstanceConfig(decoded), raw);
    assert.equal(resolved.enabled, false);
    assert.deepEqual(resolved.availability, {
      status: "unavailable",
      reason: "DRIVER_NOT_REGISTERED",
    });
  });

  it("gogoke-s1-r4/R4-01 rejects an unknown manifest major", () => {
    const valid = manifest("harness_a");
    assert.throws(
      () => parseAdapterManifest({ ...valid, schema: "gogoke.adapter-manifest.v2" }),
      (error: unknown) => error instanceof RuntimeCatalogError && error.code === "INVALID_MANIFEST",
    );
  });

  it("gogoke-s1-r4/R4-01 preserves an unknown optional capability as unknown", () => {
    const parsed = manifest("harness_a");
    assert.deepEqual(parsed.declaredCapabilities[1], {
      name: "future.optional",
      support: "unknown",
    });
  });

  it("gogoke-s1-r4/R4-01 rejects non-JSON instance config instead of asserting it", () => {
    assert.throws(
      () =>
        decodeRuntimeInstanceConfig(
          encodedInstance({ config: { nested: { impossible: Number.NaN } } }),
        ),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "INVALID_INSTANCE_CONFIG",
    );
  });

  it("gogoke-s1-r4/R4-01 keeps instance, model, seat, and account references separate", () => {
    const sharedAccount = "account_shared";
    const first = decodeRuntimeInstanceConfig(
      encodedInstance({ instanceId: "instance_a", accountRef: sharedAccount }),
    );
    const second = decodeRuntimeInstanceConfig(
      encodedInstance({ instanceId: "instance_b", accountRef: sharedAccount }),
    );
    const firstModel: ModelRef = {
      runtimeInstanceId: first.value.instanceId,
      nativeModelId: "same-native-model" as ModelRef["nativeModelId"],
      resolvedVersion: "unknown",
      capabilityRevision: "1" as ModelRef["capabilityRevision"],
    };
    const secondModel: ModelRef = {
      ...firstModel,
      runtimeInstanceId: second.value.instanceId,
    };

    assert.equal(first.value.accountRef, second.value.accountRef);
    assert.notEqual(first.value.instanceId, second.value.instanceId);
    assert.notEqual(modelIdentityKey(firstModel), modelIdentityKey(secondModel));
    assert.equal("seatId" in first.value, false);
  });

  it("gogoke-s1-r4/R4-01 requires host API, platform, services, effects, and admission", () => {
    const parsed = manifest("harness_a");
    const base = {
      hostApiVersion: "1.2.0",
      hostPlatform: "win32",
      availableHostServices: new Set(["process.spawn"]),
      allowedEffects: new Set(["process.spawn"]),
      admittedAdmissionRefs: new Set([parsed.admissionRef]),
    };
    const construct = (overrides: Partial<typeof base>) =>
      new RuntimeCatalog([{ manifest: parsed }], { ...base, ...overrides });

    assert.throws(
      () => construct({ hostApiVersion: "2.0.0" }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "HOST_API_INCOMPATIBLE",
    );
    assert.throws(
      () => construct({ hostPlatform: "linux" }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "PLATFORM_UNSUPPORTED",
    );
    assert.throws(
      () => construct({ availableHostServices: new Set() }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "MISSING_HOST_SERVICE",
    );
    assert.throws(
      () => construct({ allowedEffects: new Set() }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "EFFECT_NOT_GRANTED",
    );
    assert.throws(
      () => construct({ admittedAdmissionRefs: new Set() }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "ADMISSION_NOT_GRANTED",
    );
  });

  it("gogoke-s1-r4/R4-01 compares every SemVer component without numeric precision loss", () => {
    const cases = [
      {
        min: "9007199254740992.0.0",
        current: "9007199254740992.0.0",
        max: "9007199254740993.0.0",
      },
      {
        min: "1.9007199254740992.0",
        current: "1.9007199254740992.0",
        max: "1.9007199254740993.0",
      },
      {
        min: "1.0.9007199254740992",
        current: "1.0.9007199254740992",
        max: "1.0.9007199254740993",
      },
      { min: "1.2.0", current: "1.9.0", max: "1.10.0" },
    ];

    for (const [index, values] of cases.entries()) {
      const parsed = manifest(`semver_${index}`, "1.0.0", digest("a"), {
        minInclusive: values.min,
        maxExclusive: values.max,
      });
      const context = {
        hostApiVersion: values.current,
        hostPlatform: "win32",
        availableHostServices: new Set(["process.spawn"]),
        allowedEffects: new Set(["process.spawn"]),
        admittedAdmissionRefs: new Set([parsed.admissionRef]),
      };

      assert.doesNotThrow(() => new RuntimeCatalog([{ manifest: parsed }], context));
      assert.throws(
        () =>
          new RuntimeCatalog([{ manifest: parsed }], {
            ...context,
            hostApiVersion: values.max,
          }),
        (error: unknown) =>
          error instanceof RuntimeCatalogError && error.code === "HOST_API_INCOMPATIBLE",
      );
    }
  });

  it("gogoke-s1-r4/R4-01 snapshots manifests without executing active object properties", () => {
    const valid = manifest("harness_a");
    let driverReads = 0;
    const driverGetter = { ...valid } as Record<string, unknown>;
    Object.defineProperty(driverGetter, "driverId", {
      enumerable: true,
      get: () => {
        driverReads += 1;
        return "forged_driver";
      },
    });
    assert.throws(() => parseAdapterManifest(driverGetter), RuntimeCatalogError);
    assert.equal(driverReads, 0);

    let nestedReads = 0;
    const sourceGetter: Record<string, unknown> = { kind: "bundled" };
    Object.defineProperty(sourceGetter, "provenanceRef", {
      enumerable: true,
      get: () => {
        nestedReads += 1;
        return "source://forged";
      },
    });
    assert.throws(
      () => parseAdapterManifest({ ...valid, source: sourceGetter }),
      RuntimeCatalogError,
    );
    assert.equal(nestedReads, 0);

    let proxyReads = 0;
    const proxied = new Proxy(
      { ...valid },
      {
        get: (target, property, receiver) => {
          proxyReads += 1;
          return Reflect.get(target, property, receiver);
        },
      },
    );
    assert.throws(() => parseAdapterManifest(proxied), RuntimeCatalogError);
    assert.equal(proxyReads, 0);

    class ActiveSource {
      readonly kind = "bundled";
      readonly provenanceRef = "source://class-instance";
    }
    assert.throws(
      () => parseAdapterManifest({ ...valid, source: new ActiveSource() }),
      RuntimeCatalogError,
    );
    class ActivePlatforms extends Array<string> {}
    assert.throws(
      () => parseAdapterManifest({ ...valid, platforms: new ActivePlatforms("win32") }),
      RuntimeCatalogError,
    );
    assert.throws(
      () => parseAdapterManifest({ ...valid, futureHook: () => undefined }),
      RuntimeCatalogError,
    );
    const extendedPlatforms = ["win32"] as unknown as Record<string, unknown>;
    Object.defineProperty(extendedPlatforms, "4294967295", {
      value: () => "forged",
      enumerable: true,
    });
    assert.throws(
      () => parseAdapterManifest({ ...valid, platforms: extendedPlatforms }),
      RuntimeCatalogError,
    );

    const nullPrototypeManifest = Object.assign(Object.create(null), {
      ...valid,
      source: Object.assign(Object.create(null), valid.source),
    });
    assert.equal(parseAdapterManifest(nullPrototypeManifest).driverId, "harness_a");
  });

  it("gogoke-s1-r4/R4-02 registers a post-build random legal id without a core brand branch", () => {
    const driverId = `mock_novel_${randomBytes(6).toString("hex")}`;
    const novel = manifest(driverId);
    const runtimeCatalog = catalog([novel]);
    const decoded = decodeRuntimeInstanceConfig(
      encodedInstance({ driverId, instanceId: `${driverId}_one` }),
    );

    const resolved = runtimeCatalog.resolveInstance(decoded);
    assert.equal(resolved.availability.status, "available");
    assert.equal(resolved.enabled, false);

    const afterUninstall = catalog([]).resolveInstance(decoded);
    assert.equal(afterUninstall.availability.status, "unavailable");
    assert.deepEqual(encodeRuntimeInstanceConfig(afterUninstall.decoded), {
      ...encodedInstance({ driverId, instanceId: `${driverId}_one` }),
    });

    const afterReinstall = catalog([novel]).resolveInstance(afterUninstall.decoded);
    assert.equal(afterReinstall.availability.status, "available");
    assert.deepEqual(afterReinstall.decoded.value.config, decoded.value.config);
  });

  it("gogoke-s1-r4/R4-03 does not promote declared or observed support to qualified", () => {
    const declared = manifest("harness_a").declaredCapabilities;
    let layers = createCapabilityLayers(declared);
    const qualification = qualificationInput();

    assert.deepEqual(resolveQualifiedCapability(layers, "prompt", qualification), {
      support: "unknown",
      layer: "none",
      reason: "NOT_QUALIFIED",
      evidenceRefs: [],
    });

    layers = withObservedCapability(layers, {
      name: "prompt",
      support: "supported",
      evidenceRefs: ["observation://fixture"],
    });
    assert.equal(
      resolveQualifiedCapability(layers, "prompt", qualification).reason,
      "NOT_QUALIFIED",
    );

    layers = withQualifiedCapability(layers, {
      name: "prompt",
      support: "supported",
      evidenceRefs: ["qualification://fixture"],
      qualification,
    });
    assert.equal(resolveQualifiedCapability(layers, "prompt", qualification).support, "supported");

    const changedModel = qualificationInput({ resolvedModelVersion: "2026-09-02" });
    assert.equal(
      resolveQualifiedCapability(layers, "prompt", changedModel).reason,
      "STALE_QUALIFICATION",
    );
  });

  it("gogoke-s1-r4/R4-03 keeps qualifications for multiple instances independently", () => {
    const firstQualification = qualificationInput({
      runtimeInstanceId: "instance_a" as RuntimeInstanceId,
      resolvedModelVersion: "unknown",
    });
    const secondQualification = qualificationInput({
      runtimeInstanceId: "instance_b" as RuntimeInstanceId,
      resolvedModelVersion: "unknown",
    });
    let layers = createCapabilityLayers([]);
    layers = withQualifiedCapability(layers, {
      name: "prompt",
      support: "supported",
      evidenceRefs: ["qualification://first"],
      qualification: firstQualification,
    });
    layers = withQualifiedCapability(layers, {
      name: "prompt",
      support: "unsupported",
      evidenceRefs: ["qualification://second"],
      qualification: secondQualification,
    });

    assert.equal(
      resolveQualifiedCapability(layers, "prompt", firstQualification).support,
      "supported",
    );
    assert.equal(
      resolveQualifiedCapability(layers, "prompt", secondQualification).support,
      "unsupported",
    );
  });

  it("gogoke-s1-r4/R4-03 clones and deeply freezes capability evidence and keys", () => {
    const constraints = { modes: ["managed"] };
    const evidenceRefs = ["qualification://original"];
    const qualification = qualificationInput();
    let layers = createCapabilityLayers([{ name: "prompt", support: "supported", constraints }]);
    layers = withObservedCapability(layers, {
      name: "prompt",
      support: "supported",
      constraints,
      evidenceRefs,
    });
    layers = withQualifiedCapability(layers, {
      name: "prompt",
      support: "supported",
      constraints,
      evidenceRefs,
      qualification,
    });

    constraints.modes.push("forged");
    evidenceRefs.push("qualification://forged");
    (qualification as { profileRef: string }).profileRef = "profile://forged";

    assert.deepEqual(layers.declared[0]?.constraints, { modes: ["managed"] });
    assert.deepEqual(layers.observed[0]?.constraints, { modes: ["managed"] });
    assert.deepEqual(layers.observed[0]?.evidenceRefs, ["qualification://original"]);
    assert.deepEqual(layers.qualified[0]?.constraints, { modes: ["managed"] });
    assert.deepEqual(layers.qualified[0]?.evidenceRefs, ["qualification://original"]);
    assert.equal(layers.qualified[0]?.qualification.profileRef, "profile://one");
    assert.equal(Object.isFrozen(layers), true);
    assert.equal(Object.isFrozen(layers.qualified), true);
    assert.equal(Object.isFrozen(layers.qualified[0]), true);
    assert.equal(Object.isFrozen(layers.declared[0]?.constraints), true);
    assert.equal(Object.isFrozen(layers.observed[0]?.evidenceRefs), true);
    assert.equal(Object.isFrozen(layers.qualified[0]?.evidenceRefs), true);
    assert.equal(Object.isFrozen(layers.qualified[0]?.qualification), true);
  });

  it("gogoke-s1-r4/R4-03 rejects non-data, duplicate, and extended qualification inputs", () => {
    const first = {
      ...qualificationInput(),
      futureAuthorityRevision: "A",
    } as unknown as QualificationKeyInput;
    const second = {
      ...qualificationInput(),
      futureAuthorityRevision: "B",
    } as unknown as QualificationKeyInput;
    assert.throws(() => makeQualificationKey(first), RuntimeCatalogError);
    assert.throws(() => makeQualificationKey(second), RuntimeCatalogError);

    const withSymbol = qualificationInput() as QualificationKeyInput & Record<symbol, string>;
    Object.defineProperty(withSymbol, Symbol("authority"), {
      value: "forged",
      enumerable: true,
    });
    assert.throws(() => makeQualificationKey(withSymbol), RuntimeCatalogError);

    let driverReads = 0;
    const accessor = qualificationInput() as unknown as Record<string, unknown>;
    Object.defineProperty(accessor, "driverId", {
      enumerable: true,
      get: () => {
        driverReads += 1;
        return "forged_driver";
      },
    });
    assert.throws(
      () => makeQualificationKey(accessor as unknown as QualificationKeyInput),
      RuntimeCatalogError,
    );
    assert.equal(driverReads, 0);

    assert.throws(
      () =>
        createCapabilityLayers([
          { name: "prompt", support: "supported" },
          { name: "prompt", support: "unsupported" },
        ]),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "INVALID_CAPABILITY",
    );

    let claimReads = 0;
    const claim: Record<string, unknown> = { support: "supported" };
    Object.defineProperty(claim, "name", {
      enumerable: true,
      get: () => {
        claimReads += 1;
        return "prompt";
      },
    });
    assert.throws(
      () => createCapabilityLayers([claim as unknown as { name: string; support: "supported" }]),
      RuntimeCatalogError,
    );
    assert.equal(claimReads, 0);
  });

  it("gogoke-s1-r4/R4-03 rejects arbitrary keys and unknown capability qualification", () => {
    const qualification = qualificationInput();
    const authentic = createCapabilityLayers([]);
    const cloned = { ...authentic };
    assert.throws(
      () =>
        withObservedCapability(cloned, {
          name: "prompt",
          support: "supported",
          evidenceRefs: ["observation://clone"],
        }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_CAPABILITY_LAYERS",
    );
    assert.throws(
      () =>
        withQualifiedCapability(cloned, {
          name: "prompt",
          support: "supported",
          evidenceRefs: ["qualification://clone"],
          qualification,
        }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_CAPABILITY_LAYERS",
    );
    assert.throws(
      () => resolveQualifiedCapability(cloned, "prompt", qualification),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_CAPABILITY_LAYERS",
    );
    assert.throws(
      () =>
        withObservedCapability(createCapabilityLayers([]), {
          name: "prompt",
          support: "supported",
          evidenceRefs: [],
        }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "INVALID_CAPABILITY",
    );
    const observed = withObservedCapability(createCapabilityLayers([]), {
      name: "future.injected",
      support: "supported",
      evidenceRefs: ["observation://future"],
    });
    assert.equal(observed.observed[0]?.support, "unknown");
    assert.throws(
      () =>
        withQualifiedCapability(createCapabilityLayers([]), {
          name: "future.injected",
          support: "supported",
          evidenceRefs: ["qualification://forged"],
          qualification,
        }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "INVALID_CAPABILITY",
    );

    const arbitraryLayers = {
      declared: [],
      observed: [],
      qualified: [
        {
          name: "prompt",
          support: "supported" as const,
          evidenceRefs: ["qualification://forged"],
          qualification,
          qualificationKey: "attacker-controlled",
        },
      ],
    };
    assert.throws(
      () => resolveQualifiedCapability(arbitraryLayers, "prompt", qualification),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_CAPABILITY_LAYERS",
    );
    assert.throws(
      () => resolveQualifiedCapability(arbitraryLayers, "future.injected", qualification),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_CAPABILITY_LAYERS",
    );
  });

  it("gogoke-s1-r4/R4-03 accepts leases only from this catalog's exact resolved object", () => {
    const known = manifest("harness_a");
    const runtimeCatalog = catalog([known]);
    const decoded = decodeRuntimeInstanceConfig(encodedInstance({ enabled: true }));
    assert.throws(
      () => encodeRuntimeInstanceConfig({ ...decoded }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_INSTANCE_CONFIG",
    );
    assert.throws(
      () => runtimeCatalog.resolveInstance({ ...decoded }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_INSTANCE_CONFIG",
    );
    const resolved = runtimeCatalog.resolveInstance(decoded);
    assert.equal(runtimeCatalog.initialLeaseState(resolved).phase, "ready");
    assert.throws(
      () => runtimeCatalog.initialLeaseState({ ...resolved }),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_RESOLVED_INSTANCE",
    );

    const forgedManifest = manifest("harness_a", "9.9.9", digest("f"));
    const forgedDecoded = decodeRuntimeInstanceConfig(
      encodedInstance({ adapterVersion: "9.9.9", enabled: true }),
    );
    const forged = {
      decoded: forgedDecoded,
      enabled: true,
      availability: {
        status: "available",
        registration: { manifest: forgedManifest },
      },
    } as ResolvedRuntimeInstanceConfig;
    assert.throws(
      () => runtimeCatalog.initialLeaseState(forged),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_RESOLVED_INSTANCE",
    );

    const foreignResolved = catalog([forgedManifest]).resolveInstance(forgedDecoded);
    assert.throws(
      () => runtimeCatalog.initialLeaseState(foreignResolved),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_RESOLVED_INSTANCE",
    );
  });

  it("gogoke-s1-r4/R4-03 keeps lease ownership catalog-specific across standalone transitions", () => {
    const first = manifest("harness_a", "1.0.0", digest("a"));
    const second = manifest("harness_a", "2.0.0", digest("b"));
    const catalogA = catalog([first]);
    const catalogB = catalog([first, second]);
    const decoded = decodeRuntimeInstanceConfig(encodedInstance({ enabled: true }));
    const leaseA = catalogA.initialLeaseState(catalogA.resolveInstance(decoded));
    const boundA = openBinding(leaseA, "binding-a");

    assert.throws(
      () => catalogB.requestAdapterSwitch(leaseA, "2.0.0"),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_LEASE_STATE",
    );
    assert.throws(
      () => catalogB.requestAdapterSwitch(boundA, "2.0.0"),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_LEASE_STATE",
    );
    assert.equal(closeBinding(boundA, "binding-a").currentAdapter?.adapterVersion, "1.0.0");
  });

  it("gogoke-s1-r4/R4-03 drains bindings before switching parser identity", () => {
    const first = manifest("harness_a", "1.0.0", digest("a"));
    const second = manifest("harness_a", "2.0.0", digest("b"));
    const third = manifest("harness_a", "3.0.0", digest("c"));
    const runtimeCatalog = catalog([first, second, third]);
    const resolved = runtimeCatalog.resolveInstance(
      decodeRuntimeInstanceConfig(encodedInstance({ enabled: true })),
    );
    const active = openBinding(runtimeCatalog.initialLeaseState(resolved), "binding-one");
    const draining = runtimeCatalog.requestAdapterSwitch(active, "2.0.0");

    assert.equal(draining.phase, "draining");
    assert.equal(draining.currentAdapter?.adapterVersion, "1.0.0");
    assert.equal(draining.pendingAdapter?.adapterVersion, "2.0.0");
    assert.equal(Object.isFrozen(active), true);
    assert.equal(Object.isFrozen(active.activeBindings), true);
    assert.equal(Object.isFrozen(draining), true);
    assert.equal(Object.isFrozen(draining.currentAdapter), true);
    assert.equal(Object.isFrozen(draining.pendingAdapter), true);
    assert.equal(Object.isFrozen(draining.activeBindings), true);
    assert.throws(() => {
      (draining.pendingAdapter as { adapterVersion: string }).adapterVersion = "3.0.0";
    }, TypeError);
    assert.throws(() => {
      (draining.activeBindings as string[]).push("forged-binding");
    }, TypeError);
    assert.throws(
      () => openBinding({ ...active }, "forged-binding"),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_LEASE_STATE",
    );
    const forgedDraining = {
      ...draining,
      pendingAdapter: {
        ...draining.pendingAdapter!,
        adapterVersion: "9.9.9",
      },
    };
    assert.throws(
      () => closeBinding(forgedDraining, "binding-one"),
      (error: unknown) =>
        error instanceof RuntimeCatalogError && error.code === "UNTRUSTED_LEASE_STATE",
    );
    assert.throws(() => openBinding(draining, "binding-two"), RuntimeCatalogError);
    assert.throws(
      () => runtimeCatalog.requestAdapterSwitch(draining, "3.0.0"),
      RuntimeCatalogError,
    );

    const switched = closeBinding(draining, "binding-one");
    assert.equal(switched.phase, "ready");
    assert.equal(switched.currentAdapter?.adapterVersion, "2.0.0");
    assert.equal(switched.pendingAdapter, null);
  });
});
