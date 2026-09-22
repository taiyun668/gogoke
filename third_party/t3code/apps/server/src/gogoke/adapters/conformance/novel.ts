import type { JsonObject, RuntimeDriverId, RuntimeInstanceId } from "../../contracts/model.ts";
import {
  closeBinding,
  decodeRuntimeInstanceConfig,
  encodeRuntimeInstanceConfig,
  openBinding,
  parseAdapterManifest,
  RuntimeCatalog,
  RUNTIME_INSTANCE_CONFIG_SCHEMA,
  type AdapterManifestV1,
  type RuntimeCatalogHostContext,
} from "../../runtimeCatalog/index.ts";

const SHA256 = /^sha256:[0-9a-f]{64}$/u;

export interface NovelDriverConformanceReceipt {
  readonly coreBuildDigest: string;
  readonly driverId: RuntimeDriverId;
  readonly initialPhase: "ready";
  readonly switchingPhase: "draining";
  readonly switchedVersion: "2.0.0";
  readonly unavailableReason: "DRIVER_NOT_REGISTERED";
  readonly reinstalled: true;
  readonly encodedConfig: JsonObject;
}

const manifest = (
  driverId: RuntimeDriverId,
  adapterVersion: "1.0.0" | "2.0.0",
  digestCharacter: "a" | "b",
): AdapterManifestV1 =>
  parseAdapterManifest({
    schema: "gogoke.adapter-manifest.v1",
    packageId: `fixture.${driverId}`,
    driverId,
    adapterVersion,
    artifactDigest: `sha256:${digestCharacter.repeat(64)}`,
    hostApiRange: { minInclusive: "1.0.0", maxExclusive: "2.0.0" },
    nativeProtocol: "fixture-jsonl",
    platforms: ["win32"],
    configSchemaRef: `schema://${driverId}/config/v1`,
    declaredCapabilities: [{ name: "prompt", support: "supported" }],
    requiredHostServices: ["process.spawn"],
    requestedEffects: ["process.spawn"],
    source: { kind: "bundled", provenanceRef: `fixture://${driverId}` },
    license: { spdxId: "MIT", noticeRef: `notice://${driverId}` },
    admissionRef: `admission://${driverId}`,
  });

const hostContext = (manifests: ReadonlyArray<AdapterManifestV1>): RuntimeCatalogHostContext => ({
  hostApiVersion: "1.2.0",
  hostPlatform: "win32",
  availableHostServices: new Set(["process.spawn"]),
  allowedEffects: new Set(["process.spawn"]),
  admittedAdmissionRefs: new Set(manifests.map((entry) => entry.admissionRef)),
});

const catalog = (manifests: ReadonlyArray<AdapterManifestV1>): RuntimeCatalog =>
  new RuntimeCatalog(
    manifests.map((entry) => ({ manifest: entry })),
    hostContext(manifests),
  );

/** Runs after the caller has fixed the core build digest; the novel ID is selected here. */
export function runNovelDriverConformance(
  coreBuildDigest: string,
  randomBytes: (size: number) => Uint8Array,
): NovelDriverConformanceReceipt {
  if (!SHA256.test(coreBuildDigest)) throw new Error("INVALID_CORE_BUILD_DIGEST");
  const entropy = Uint8Array.from(randomBytes(8));
  if (entropy.byteLength !== 8) throw new Error("INVALID_NOVEL_DRIVER_ENTROPY");
  const driverId = `mock_novel_${Buffer.from(entropy).toString("hex")}` as RuntimeDriverId;
  const first = manifest(driverId, "1.0.0", "a");
  const second = manifest(driverId, "2.0.0", "b");
  const rawConfig = {
    schema: RUNTIME_INSTANCE_CONFIG_SCHEMA,
    instanceId: `${driverId}_instance` as RuntimeInstanceId,
    driverId,
    adapterVersion: "1.0.0",
    enabled: true,
    config: { executable: "fixture.exe", opaque: { retain: true } },
    futureConfigRevision: "retained",
  };
  const decoded = decodeRuntimeInstanceConfig(rawConfig);
  const installed = catalog([first, second]);
  const initial = installed.initialLeaseState(installed.resolveInstance(decoded));
  if (initial.phase !== "ready") throw new Error("NOVEL_DRIVER_NOT_READY");
  const active = openBinding(initial, "binding-one");
  const switching = installed.requestAdapterSwitch(active, "2.0.0");
  if (switching.phase !== "draining") throw new Error("NOVEL_DRIVER_DID_NOT_DRAIN");
  const switched = closeBinding(switching, "binding-one");
  if (switched.currentAdapter?.adapterVersion !== "2.0.0") {
    throw new Error("NOVEL_DRIVER_SWITCH_FAILED");
  }

  const unavailable = catalog([]).resolveInstance(decoded);
  if (
    unavailable.availability.status !== "unavailable" ||
    unavailable.availability.reason !== "DRIVER_NOT_REGISTERED"
  ) {
    throw new Error("NOVEL_DRIVER_UNINSTALL_FAILED");
  }
  const encodedConfig = encodeRuntimeInstanceConfig(unavailable.decoded);
  const reinstalled = catalog([first]).resolveInstance(unavailable.decoded);
  if (reinstalled.availability.status !== "available") {
    throw new Error("NOVEL_DRIVER_REINSTALL_FAILED");
  }

  return Object.freeze({
    coreBuildDigest,
    driverId,
    initialPhase: "ready",
    switchingPhase: "draining",
    switchedVersion: "2.0.0",
    unavailableReason: "DRIVER_NOT_REGISTERED",
    reinstalled: true,
    encodedConfig,
  });
}
