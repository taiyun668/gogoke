import * as NodeAssert from "node:assert/strict";
import * as NodeChildProcess from "node:child_process";
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeSqlite from "node:sqlite";
import * as NodeTest from "node:test";

import {
  canonicalJson,
  type JsonObject,
  type JsonValue,
  type PublicObjectType,
} from "../../contracts/index.ts";
import {
  applyCoreMigration,
  CORE_MIGRATION_IDENTITY,
  CORE_TABLE_NAMES,
  type CoreIndexName,
  type CoreMigrationDriver,
  type CoreMigrationTransaction,
  type CoreSchemaSnapshot,
  type CoreTableName,
} from "../../migrations/core/index.ts";
import {
  AtomicCommitUnknownError,
  AtomicRecordError,
  OBJECT_ROW_BINDING_TYPES,
  type AtomicRecordInput,
  type AtomicSqliteDriver,
  type AtomicSqliteTransaction,
  type AtomicSqlValue,
  type ExistingAtomicReceipt,
  writeAtomicRecord,
} from "./atomicRecord.ts";
import * as BaseExports from "./index.ts";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

const jsonBytes = (value: JsonValue): Uint8Array => new TextEncoder().encode(canonicalJson(value));

interface PublicObjectCase {
  readonly objectType: PublicObjectType;
  readonly object: JsonObject;
  readonly domainId: string;
  readonly expectedId: ReadonlyArray<JsonValue>;
  readonly expectedVersion: ReadonlyArray<JsonValue>;
  readonly nativeIdentity?: {
    readonly runtimeInstanceId: string;
    readonly nativeId: string;
  };
}

const PUBLIC_OBJECT_CASES: ReadonlyArray<PublicObjectCase> = [
  {
    objectType: "RuntimeDriver",
    object: {
      driverId: "driver-a",
      adapterVersion: "1.0.0",
      artifactDigest: "sha256:driver",
      configSchemaRef: "schema:driver",
      requiredHostServices: ["process"],
      admissionRef: "admission-driver",
    },
    domainId: "domain-public",
    expectedId: ["driver-a"],
    expectedVersion: ["1.0.0"],
  },
  {
    objectType: "RuntimeInstance",
    object: {
      instanceId: "instance-a",
      driverId: "driver-a",
      binaryIdentity: "binary-a",
      profileRef: "profile-a",
      authRevision: "1",
      hostRef: "host-a",
      capacityPoolRef: "pool-a",
    },
    domainId: "domain-public",
    expectedId: ["instance-a"],
    expectedVersion: ["1"],
  },
  {
    objectType: "ModelRef",
    object: {
      runtimeInstanceId: "instance-a",
      nativeModelId: "model-a",
      resolvedVersion: "2026-09",
      capabilityRevision: "2",
    },
    domainId: "domain-public",
    expectedId: ["instance-a", "model-a"],
    expectedVersion: ["2026-09", "2"],
  },
  {
    objectType: "RoleSpec",
    object: {
      roleId: "role-a",
      revision: "3",
      responsibility: "build",
      requiredCapabilities: ["code"],
      delegableCeiling: { level: "bounded" },
      reviewIndependence: true,
    },
    domainId: "domain-public",
    expectedId: ["role-a"],
    expectedVersion: ["3"],
  },
  {
    objectType: "Seat",
    object: {
      seatId: "seat-a",
      roleId: "role-a",
      scope: "project",
      domainId: "domain-seat",
      lifecycle: "active",
      grantRef: "grant-a",
    },
    domainId: "domain-seat",
    expectedId: ["seat-a"],
    expectedVersion: ["active"],
  },
  {
    objectType: "ExecutionRecipe",
    object: {
      recipeId: "recipe-a",
      revision: "4",
      seatId: "seat-a",
      runtimeInstanceId: "instance-a",
      modelRef: { nativeModelId: "model-a" },
      toolProfile: { name: "tools-a" },
      isolationProfile: { name: "isolated" },
      contextManifestId: "manifest-a",
      budgetPolicy: { max: "10" },
      admissionRef: "admission-a",
    },
    domainId: "domain-public",
    expectedId: ["recipe-a"],
    expectedVersion: ["4"],
  },
  {
    objectType: "NativeBinding",
    object: {
      bindingId: "binding-a",
      generation: "5",
      sourceEpoch: "6",
      instanceId: "instance-a",
      nativeIdentity: "native-a",
      domainId: "domain-binding",
      lineageRef: "lineage-a",
      custodyRef: "custody-a",
    },
    domainId: "domain-binding",
    expectedId: ["binding-a"],
    expectedVersion: ["5"],
    nativeIdentity: { runtimeInstanceId: "instance-a", nativeId: "native-a" },
  },
  {
    objectType: "ContextObject",
    object: {
      contextId: "context-a",
      version: "7",
      scope: "PROJECT",
      domainId: "domain-context",
      kind: "fact",
      contentHash: "sha256:context",
      sourceRef: { id: "source-a" },
      sourceAuthority: { level: "owner" },
      derivedFrom: ["source-a"],
      validity: "ACTIVE",
      supersedes: [],
      accessPolicyRevision: "8",
    },
    domainId: "domain-context",
    expectedId: ["context-a"],
    expectedVersion: ["7", "8"],
  },
  {
    objectType: "ContextManifest",
    object: {
      manifestId: "manifest-a",
      taskId: "task-a",
      seatId: "seat-a",
      bindingGeneration: "9",
      domainId: "domain-manifest",
      policyRevision: "10",
      sourceSnapshot: { hash: "source" },
      requiredConstraints: [{ id: "constraint-a" }],
      includedVersions: [{ id: "context-a", version: "7" }],
      redactions: [],
      selectionDecisionId: "decision-a",
      manifestHash: "sha256:manifest",
    },
    domainId: "domain-manifest",
    expectedId: ["manifest-a"],
    expectedVersion: ["9", "10", "sha256:manifest"],
  },
  {
    objectType: "ExposureReceipt",
    object: {
      receiptId: "exposure-a",
      manifestId: "manifest-a",
      bindingId: "binding-a",
      generation: "11",
      evidenceLevel: "NATIVE_ACKED",
      nativeSourceCoverage: { complete: true },
      taintLabels: [],
      evidenceRefs: ["evidence-a"],
    },
    domainId: "domain-public",
    expectedId: ["exposure-a"],
    expectedVersion: ["11"],
  },
  {
    objectType: "SessionLineage",
    object: {
      sessionId: "session-a",
      bindingId: "binding-a",
      parentRefs: [],
      operationKind: "new",
      inheritedExposure: { labels: [] },
      sourceEpoch: "12",
    },
    domainId: "domain-public",
    expectedId: ["session-a"],
    expectedVersion: ["12"],
  },
  {
    objectType: "DecisionRecord",
    object: {
      decisionId: "decision-a",
      family: "routing",
      stateViewHash: "sha256:state",
      candidateHash: "sha256:candidates",
      sourceRevisions: { policy: "1" },
      backend: "rules",
      modelRequested: "none",
      modelResolved: "none",
      questionVersion: "q1",
      probabilities: { choose: 1 },
      nativeConfidence: null,
      calibrationRef: "calibration-a",
      mode: "deterministic",
      state: "COMMITTED",
      actionId: "action-a",
    },
    domainId: "domain-public",
    expectedId: ["decision-a"],
    expectedVersion: ["sha256:state", "sha256:candidates", "q1", "COMMITTED"],
  },
  {
    objectType: "OutcomeRecord",
    object: {
      outcomeId: "outcome-a",
      decisionId: "decision-a",
      actionId: "action-a",
      revision: "13",
      labelSource: "downstream",
      evidenceRefs: ["evidence-a"],
      observationWindow: { end: "later" },
      censorStatus: "OBSERVED",
      quality: { score: 1 },
      cost: { units: 1 },
      latency: { milliseconds: 2 },
      rework: { count: 0 },
      safetyEvents: [],
    },
    domainId: "domain-public",
    expectedId: ["outcome-a"],
    expectedVersion: ["13"],
  },
  {
    objectType: "CalibrationProfile",
    object: {
      profileId: "calibration-a",
      family: "routing",
      modelVersion: "model-v1",
      questionViewVersion: "view-v1",
      locale: "en-US",
      taskDomain: "coding",
      datasetSplitHash: "sha256:split",
      rubricHash: "sha256:rubric",
      sampleCounts: { train: 10 },
      uncertaintyIntervals: { low: 0.1, high: 0.2 },
      coverageRisk: { level: "low" },
      testOnly: true,
      qualificationGrant: { id: "grant-a" },
    },
    domainId: "domain-public",
    expectedId: ["calibration-a"],
    expectedVersion: ["model-v1", "view-v1", "sha256:split", "sha256:rubric"],
  },
  {
    objectType: "DreamRun",
    object: {
      runId: "dream-run-a",
      snapshotHash: "sha256:snapshot",
      domainId: "domain-dream",
      budgetLease: { units: 1 },
      stepReceipts: ["receipt-a"],
      preemptionState: "idle",
      datasetSplitHash: "sha256:dream-split",
      recipeRef: "recipe-a",
    },
    domainId: "domain-dream",
    expectedId: ["dream-run-a"],
    expectedVersion: ["sha256:snapshot", "sha256:dream-split", "idle"],
  },
  {
    objectType: "DreamProposal",
    object: {
      proposalId: "proposal-a",
      runId: "dream-run-a",
      kind: "memory-candidate",
      basePolicyRevision: "14",
      candidateHash: "sha256:proposal",
      allowedChangeSet: { paths: ["memory"] },
      evaluationRefs: ["evaluation-a"],
      heldoutReceipt: "heldout-a",
      state: "DEV_VALIDATED",
      activationGrant: { id: "grant-a" },
      rollbackRef: "rollback-a",
    },
    domainId: "domain-public",
    expectedId: ["proposal-a"],
    expectedVersion: ["14", "sha256:proposal", "DEV_VALIDATED"],
  },
];

function publicObjectBytes(objectCase: PublicObjectCase): Uint8Array {
  return jsonBytes({
    object: objectCase.object,
    objectType: objectCase.objectType,
    schema: "gogoke.s1-r4.objects.v1",
  });
}

function request(
  objectCase: PublicObjectCase,
  suffix: string,
  overrides: {
    readonly streamId?: string;
    readonly expectedPreviousCounter?: string | null;
    readonly counter?: string;
    readonly operationId?: string;
    readonly receiptPayload?: JsonValue;
  } = {},
): AtomicRecordInput {
  return {
    domainId: objectCase.domainId,
    object: { canonicalBytes: publicObjectBytes(objectCase) },
    ...(objectCase.nativeIdentity === undefined
      ? {}
      : { nativeIdentity: { ...objectCase.nativeIdentity } }),
    event: {
      eventId: `event-${suffix}`,
      streamId: overrides.streamId ?? `stream-${suffix}`,
      expectedPreviousCounter: overrides.expectedPreviousCounter ?? null,
      counter: overrides.counter ?? "0",
      eventType: "object.persisted",
      occurredAt: "2026-09-20T12:00:00.000Z",
      canonicalBytes: jsonBytes({ objectType: objectCase.objectType, suffix }),
    },
    receipt: {
      receiptId: `receipt-${suffix}`,
      operationId: overrides.operationId ?? `operation-${suffix}`,
      receiptType: "persisted",
      recordedAt: "2026-09-20T12:00:00.001Z",
      canonicalBytes: jsonBytes(
        overrides.receiptPayload ?? { eventId: `event-${suffix}`, status: "persisted" },
      ),
    },
  };
}

const readField = (row: unknown, field: string): unknown => {
  if (typeof row !== "object" || row === null) throw new Error(`${field} row is not an object`);
  return Reflect.get(row, field);
};

const readStringField = (row: unknown, field: string): string => {
  const value = readField(row, field);
  if (typeof value !== "string") throw new Error(`${field} is not a string`);
  return value;
};

const readNumberField = (row: unknown, field: string): number => {
  const value = readField(row, field);
  if (typeof value !== "number") throw new Error(`${field} is not a number`);
  return value;
};

const readBytesField = (row: unknown, field: string): Uint8Array => {
  const value = readField(row, field);
  if (!(value instanceof Uint8Array)) throw new Error(`${field} is not a Uint8Array`);
  return value;
};

const mapCoreTables = <T>(read: (table: CoreTableName) => T): Record<CoreTableName, T> => ({
  gogoke_objects: read("gogoke_objects"),
  gogoke_native_identities: read("gogoke_native_identities"),
  gogoke_stream_heads: read("gogoke_stream_heads"),
  gogoke_events: read("gogoke_events"),
  gogoke_receipts: read("gogoke_receipts"),
});

const mapCoreIndexes = <T>(read: (index: CoreIndexName) => T): Record<CoreIndexName, T> => ({
  idx_gogoke_events_stream: read("idx_gogoke_events_stream"),
  idx_gogoke_receipts_event: read("idx_gogoke_receipts_event"),
});

const isPublicObjectType = (value: unknown): value is PublicObjectType =>
  typeof value === "string" && OBJECT_ROW_BINDING_TYPES.some((objectType) => objectType === value);

function readExistingReceipt(row: unknown): ExistingAtomicReceipt {
  const objectType = readField(row, "objectType");
  if (!isPublicObjectType(objectType)) throw new Error("objectType is not a public object type");
  return {
    domainId: readStringField(row, "domainId"),
    receiptId: readStringField(row, "receiptId"),
    operationId: readStringField(row, "operationId"),
    eventId: readStringField(row, "eventId"),
    objectType,
    objectId: readStringField(row, "objectId"),
    objectVersion: readStringField(row, "objectVersion"),
    objectHash: readStringField(row, "objectHash"),
    eventHash: readStringField(row, "eventHash"),
    receiptHash: readStringField(row, "receiptHash"),
    operationFingerprint: readStringField(row, "operationFingerprint"),
  };
}

function schemaSnapshot(database: NodeSqlite.DatabaseSync): CoreSchemaSnapshot {
  const definition = (name: string, type: "table" | "index"): string => {
    const row = database
      .prepare("SELECT sql FROM sqlite_schema WHERE type = ? AND name = ?")
      .get(type, name);
    return row === undefined ? "" : readStringField(row, "sql");
  };
  return {
    columns: mapCoreTables((table) =>
      database
        .prepare(`PRAGMA table_info(${table})`)
        .all()
        .map((row) => readStringField(row, "name")),
    ),
    tableDefinitions: mapCoreTables((table) => definition(table, "table")),
    indexDefinitions: mapCoreIndexes((index) => definition(index, "index")),
  };
}

class ControlledTestSqliteDriver implements AtomicSqliteDriver, CoreMigrationDriver {
  readonly calls: Array<string> = [];
  readonly database: NodeSqlite.DatabaseSync;
  failBeforeReceipt = false;
  commitUnknownAfterCommit = false;
  onBegin: (() => void) | undefined;

  constructor(database: NodeSqlite.DatabaseSync) {
    this.database = database;
  }

  async withNativePinnedMigration<T>(
    _identity: typeof CORE_MIGRATION_IDENTITY,
    operation: (transaction: CoreMigrationTransaction) => Promise<T>,
  ): Promise<T> {
    this.database.exec("BEGIN IMMEDIATE");
    try {
      const result = await operation({
        execute: async (statement) => this.database.exec(statement),
        inspectSchema: async () => schemaSnapshot(this.database),
      });
      this.database.exec("COMMIT");
      return result;
    } catch (error) {
      this.database.exec("ROLLBACK");
      throw error;
    }
  }

  async withNativePinnedImmediateTransaction<T>(
    operation: (transaction: AtomicSqliteTransaction) => Promise<T>,
  ): Promise<T> {
    this.onBegin?.();
    this.database.exec("BEGIN IMMEDIATE");
    const transaction: AtomicSqliteTransaction = {
      readDurability: async () => {
        this.calls.push("durability");
        return {
          foreignKeys: this.pragma("foreign_keys") as number,
          journalMode: this.pragma("journal_mode") as string,
          synchronous: this.pragma("synchronous") as number,
        };
      },
      queryReceiptByOperation: async (domainId, operationId) => {
        this.calls.push("operation-query");
        const row = this.database
          .prepare(
            `SELECT
               r.domain_id AS domainId,
               r.receipt_id AS receiptId,
               r.operation_id AS operationId,
               r.event_id AS eventId,
               r.object_type AS objectType,
               r.object_id AS objectId,
               r.object_version AS objectVersion,
               o.content_hash AS objectHash,
               e.content_hash AS eventHash,
               r.content_hash AS receiptHash,
               r.operation_fingerprint AS operationFingerprint
             FROM gogoke_receipts r
             JOIN gogoke_events e
               ON e.domain_id = r.domain_id AND e.event_id = r.event_id
             JOIN gogoke_objects o
               ON o.domain_id = r.domain_id
              AND o.object_type = r.object_type
              AND o.object_id = r.object_id
              AND o.object_version = r.object_version
             WHERE r.domain_id = ? AND r.operation_id = ?`,
          )
          .get(domainId, operationId);
        return row === undefined ? null : readExistingReceipt(row);
      },
      execute: async (statement: string, parameters: ReadonlyArray<AtomicSqlValue>) => {
        const table =
          /(?:INSERT INTO|UPDATE)\s+(gogoke_[a-z_]+)/i.exec(statement)?.[1] ?? "unknown";
        this.calls.push(table);
        if (this.failBeforeReceipt && table === "gogoke_receipts") {
          throw new Error("injected failure before receipt");
        }
        const result = this.database.prepare(statement).run(...parameters);
        return { changes: result.changes };
      },
    };
    try {
      const result = await operation(transaction);
      this.database.exec("COMMIT");
      this.calls.push("commit");
      if (this.commitUnknownAfterCommit) {
        this.commitUnknownAfterCommit = false;
        throw new AtomicCommitUnknownError("injected unknown commit acknowledgement");
      }
      return result;
    } catch (error) {
      try {
        this.database.exec("ROLLBACK");
      } catch {
        // A committed-but-unacknowledged transaction cannot be rolled back.
      }
      this.calls.push("rollback");
      throw error;
    }
  }

  private pragma(name: string): unknown {
    const row = this.database.prepare(`PRAGMA ${name}`).get();
    if (row === undefined) throw new Error(`PRAGMA ${name} returned no row`);
    return Object.values(row)[0];
  }
}

interface TestDatabase {
  readonly directory: string;
  readonly path: string;
  readonly database: NodeSqlite.DatabaseSync;
  readonly driver: ControlledTestSqliteDriver;
}

async function openTestDatabase(): Promise<TestDatabase> {
  const directory = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "gogoke-atomic-record-"));
  const path = NodePath.join(directory, "state.sqlite");
  const database = new NodeSqlite.DatabaseSync(path);
  database.exec("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;");
  const driver = new ControlledTestSqliteDriver(database);
  await applyCoreMigration(driver);
  return { directory, path, database, driver };
}

function closeTestDatabase(testDatabase: TestDatabase): void {
  testDatabase.database.close();
  NodeFS.rmSync(testDatabase.directory, { recursive: true, force: true });
}

const count = (database: NodeSqlite.DatabaseSync, table: string): number =>
  readNumberField(database.prepare(`SELECT COUNT(*) AS count FROM ${table}`).get(), "count");

const nativeBindingCase = (
  bindingId: string,
  domainId: string,
  instanceId: string,
  nativeId: string,
): PublicObjectCase => ({
  objectType: "NativeBinding",
  object: {
    bindingId,
    generation: "1",
    sourceEpoch: "1",
    instanceId,
    nativeIdentity: nativeId,
    domainId,
    lineageRef: `lineage-${bindingId}`,
    custodyRef: `custody-${bindingId}`,
  },
  domainId,
  expectedId: [bindingId],
  expectedVersion: ["1"],
  nativeIdentity: { runtimeInstanceId: instanceId, nativeId },
});

test("object, event, and receipt commit atomically with receipt last", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const receipt = await writeAtomicRecord(
      testDatabase.driver,
      request(PUBLIC_OBJECT_CASES[6]!, "atomic"),
    );
    assert.equal(receipt.disposition, "COMMITTED");
    assert.deepEqual(testDatabase.driver.calls, [
      "durability",
      "operation-query",
      "gogoke_stream_heads",
      "gogoke_objects",
      "gogoke_native_identities",
      "gogoke_events",
      "gogoke_receipts",
      "commit",
    ]);
    testDatabase.database.close();

    const reopened = new NodeSqlite.DatabaseSync(testDatabase.path, { readOnly: true });
    try {
      assert.equal(count(reopened, "gogoke_objects"), 1);
      assert.equal(count(reopened, "gogoke_events"), 1);
      assert.equal(count(reopened, "gogoke_receipts"), 1);
    } finally {
      reopened.close();
    }
  } finally {
    NodeFS.rmSync(testDatabase.directory, { recursive: true, force: true });
  }
});

test("a failure between event and receipt rolls the complete transaction back", async () => {
  const testDatabase = await openTestDatabase();
  try {
    testDatabase.driver.failBeforeReceipt = true;
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, request(PUBLIC_OBJECT_CASES[6]!, "rollback")),
      (error: unknown) =>
        error instanceof AtomicRecordError &&
        error.code === "ATOMIC_WRITE_FAILED" &&
        error.stage === "receipt",
    );
    for (const table of CORE_TABLE_NAMES) assert.equal(count(testDatabase.database, table), 0);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("all 16 public object rows derive identity and version from contract fields", async () => {
  const testDatabase = await openTestDatabase();
  try {
    assert.equal(OBJECT_ROW_BINDING_TYPES.length, 16);
    for (const [index, objectCase] of PUBLIC_OBJECT_CASES.entries()) {
      await writeAtomicRecord(testDatabase.driver, request(objectCase, `object-${index}`));
      const row = testDatabase.database
        .prepare(
          `SELECT object_id AS objectId, object_version AS objectVersion
           FROM gogoke_objects WHERE object_type = ?`,
        )
        .get(objectCase.objectType);
      assert.equal(readStringField(row, "objectId"), canonicalJson(objectCase.expectedId));
      assert.equal(
        readStringField(row, "objectVersion"),
        canonicalJson(objectCase.expectedVersion),
      );
    }
    assert.equal(count(testDatabase.database, "gogoke_objects"), 16);
    assert.equal(count(testDatabase.database, "gogoke_native_identities"), 1);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("native identity index is exclusive to an exactly matching NativeBinding", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const runtime = PUBLIC_OBJECT_CASES[1]!;
    const nonBindingBase = request(runtime, "non-binding-native");
    const nonBinding: AtomicRecordInput = {
      ...nonBindingBase,
      nativeIdentity: { runtimeInstanceId: "instance-a", nativeId: "native-a" },
    };
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, nonBinding),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "INVALID_RECORD",
    );

    const binding = PUBLIC_OBJECT_CASES[6]!;
    const mismatchBase = request(binding, "binding-mismatch");
    const mismatched: AtomicRecordInput = {
      ...mismatchBase,
      nativeIdentity: {
        runtimeInstanceId: mismatchBase.nativeIdentity!.runtimeInstanceId,
        nativeId: "different-native",
      },
    };
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, mismatched),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "INVALID_RECORD",
    );

    const wrongDomain = request({ ...binding, domainId: "different-domain" }, "wrong-domain");
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, wrongDomain),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "INVALID_RECORD",
    );
    assert.equal(count(testDatabase.database, "gogoke_objects"), 0);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("composite native identity does not collide across domain or instance", async () => {
  const testDatabase = await openTestDatabase();
  try {
    await writeAtomicRecord(
      testDatabase.driver,
      request(nativeBindingCase("binding-a", "domain-a", "instance-a", "same"), "native-a"),
    );
    await writeAtomicRecord(
      testDatabase.driver,
      request(nativeBindingCase("binding-b", "domain-b", "instance-a", "same"), "native-b"),
    );
    await writeAtomicRecord(
      testDatabase.driver,
      request(nativeBindingCase("binding-c", "domain-a", "instance-b", "same"), "native-c"),
    );
    await assert.rejects(
      writeAtomicRecord(
        testDatabase.driver,
        request(nativeBindingCase("binding-d", "domain-a", "instance-a", "same"), "native-d"),
      ),
      (error: unknown) =>
        error instanceof AtomicRecordError &&
        error.code === "WRITE_CONFLICT" &&
        error.stage === "native-identity",
    );
    assert.equal(count(testDatabase.database, "gogoke_native_identities"), 3);
    assert.equal(count(testDatabase.database, "gogoke_receipts"), 3);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("passive snapshot rejects active or extra input without reads", async () => {
  const testDatabase = await openTestDatabase();
  try {
    let getterReads = 0;
    const getterInput = request(PUBLIC_OBJECT_CASES[6]!, "getter");
    Object.defineProperty(getterInput, "domainId", {
      enumerable: true,
      get: () => {
        getterReads += 1;
        return "volatile";
      },
    });
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, getterInput),
      (error: unknown) => error instanceof AtomicRecordError && error.stage === "snapshot",
    );
    assert.equal(getterReads, 0);

    let byteGetterReads = 0;
    const byteGetterInput = request(PUBLIC_OBJECT_CASES[6]!, "byte-getter");
    Object.defineProperty(byteGetterInput.object, "canonicalBytes", {
      enumerable: true,
      get: () => {
        byteGetterReads += 1;
        return new Uint8Array();
      },
    });
    await assert.rejects(writeAtomicRecord(testDatabase.driver, byteGetterInput));
    assert.equal(byteGetterReads, 0);

    let proxyTraps = 0;
    const proxied = new Proxy(request(PUBLIC_OBJECT_CASES[6]!, "proxy"), {
      get: (target, property, receiver) => {
        proxyTraps += 1;
        return Reflect.get(target, property, receiver);
      },
      ownKeys: (target) => {
        proxyTraps += 1;
        return Reflect.ownKeys(target);
      },
    });
    await assert.rejects(writeAtomicRecord(testDatabase.driver, proxied));
    assert.equal(proxyTraps, 0);

    const extra = request(PUBLIC_OBJECT_CASES[6]!, "extra");
    Reflect.set(extra, "extra", "not-allowed");
    await assert.rejects(writeAtomicRecord(testDatabase.driver, extra));

    const symbol = request(PUBLIC_OBJECT_CASES[6]!, "symbol");
    Reflect.set(symbol, Symbol("hidden"), true);
    await assert.rejects(writeAtomicRecord(testDatabase.driver, symbol));
    assert.equal(testDatabase.driver.calls.length, 0);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("typed-array shadow getters are rejected without being read", async () => {
  const testDatabase = await openTestDatabase();
  try {
    for (const field of ["buffer", "byteLength", "byteOffset", "length"] as const) {
      let getterReads = 0;
      const input = request(PUBLIC_OBJECT_CASES[6]!, `bytes-${field}`);
      Object.defineProperty(input.object.canonicalBytes, field, {
        configurable: true,
        enumerable: true,
        get: () => {
          getterReads += 1;
          return 0;
        },
      });
      await assert.rejects(
        writeAtomicRecord(testDatabase.driver, input),
        (error: unknown) => error instanceof AtomicRecordError && error.stage === "snapshot",
      );
      assert.equal(getterReads, 0, `${field} getter must remain unread`);
    }
    assert.equal(testDatabase.driver.calls.length, 0);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("snapshot reads caller fields once and is stable across later mutation", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const input = request(PUBLIC_OBJECT_CASES[6]!, "snapshot");
    const originalDomain = input.domainId;
    const originalBytes = Uint8Array.from(input.object.canonicalBytes);
    testDatabase.driver.onBegin = () => {
      Reflect.set(input, "domainId", "mutated-domain");
      input.object.canonicalBytes.fill(0x78);
      input.event.canonicalBytes.fill(0x79);
      input.receipt.canonicalBytes.fill(0x7a);
    };

    await writeAtomicRecord(testDatabase.driver, input);
    const row = testDatabase.database
      .prepare("SELECT domain_id AS domainId, canonical_json AS bytes FROM gogoke_objects")
      .get();
    assert.equal(readStringField(row, "domainId"), originalDomain);
    assert.deepEqual(Uint8Array.from(readBytesField(row, "bytes")), originalBytes);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("counter conflict and non-FULL sync fail before a receipt exists", async () => {
  const testDatabase = await openTestDatabase();
  try {
    await writeAtomicRecord(
      testDatabase.driver,
      request(PUBLIC_OBJECT_CASES[0]!, "counter-base", { streamId: "shared-stream" }),
    );
    await assert.rejects(
      writeAtomicRecord(
        testDatabase.driver,
        request(PUBLIC_OBJECT_CASES[1]!, "counter-conflict", {
          streamId: "shared-stream",
          expectedPreviousCounter: "7",
          counter: "8",
        }),
      ),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "COUNTER_CONFLICT",
    );
    assert.equal(count(testDatabase.database, "gogoke_objects"), 1);

    testDatabase.database.exec("PRAGMA synchronous = NORMAL");
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, request(PUBLIC_OBJECT_CASES[1]!, "weak-sync")),
      (error: unknown) =>
        error instanceof AtomicRecordError && error.code === "DURABILITY_CONTRACT_FAILED",
    );
    assert.equal(count(testDatabase.database, "gogoke_receipts"), 1);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("bad JSON is rejected before a transaction can touch the database", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const invalid = request(PUBLIC_OBJECT_CASES[6]!, "bad-json");
    const badRequest: AtomicRecordInput = {
      ...invalid,
      event: {
        ...invalid.event,
        canonicalBytes: new TextEncoder().encode('{"duplicate":1,"duplicate":2}'),
      },
    };
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, badRequest),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "INVALID_RECORD",
    );
    assert.equal(testDatabase.driver.calls.length, 0);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("base exports contain no caller-forgeable pinned identity attestation", () => {
  assert.equal("PinnedDatabaseIdentity" in BaseExports, false);
  assert.equal("requirePinnedDatabaseIdentity" in BaseExports, false);
  assert.equal("databaseIdentity" in BaseExports, false);
});

test("committed unknown outcome reconciles exact replay and rejects changed content", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const original = request(PUBLIC_OBJECT_CASES[6]!, "reconcile");
    testDatabase.driver.commitUnknownAfterCommit = true;
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, original),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "COMMIT_UNKNOWN",
    );
    assert.equal(count(testDatabase.database, "gogoke_receipts"), 1);

    testDatabase.driver.calls.length = 0;
    const reconciled = await writeAtomicRecord(testDatabase.driver, original);
    assert.equal(reconciled.disposition, "RECONCILED");
    assert.deepEqual(testDatabase.driver.calls, ["durability", "operation-query", "commit"]);
    assert.equal(count(testDatabase.database, "gogoke_objects"), 1);

    const changed = request(PUBLIC_OBJECT_CASES[6]!, "reconcile", {
      operationId: original.receipt.operationId,
      receiptPayload: { changed: true },
    });
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, changed),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "OPERATION_CONFLICT",
    );
    const changedMetadata: AtomicRecordInput = {
      ...original,
      event: { ...original.event, eventType: "different-event-type" },
    };
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, changedMetadata),
      (error: unknown) => error instanceof AtomicRecordError && error.code === "OPERATION_CONFLICT",
    );
    assert.equal(count(testDatabase.database, "gogoke_receipts"), 1);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("different operation cannot silently re-import the same object row", async () => {
  const testDatabase = await openTestDatabase();
  try {
    const objectCase = PUBLIC_OBJECT_CASES[0]!;
    await writeAtomicRecord(testDatabase.driver, request(objectCase, "import-a"));
    await assert.rejects(
      writeAtomicRecord(testDatabase.driver, request(objectCase, "import-b")),
      (error: unknown) =>
        error instanceof AtomicRecordError &&
        error.code === "WRITE_CONFLICT" &&
        error.stage === "object",
    );
    assert.equal(count(testDatabase.database, "gogoke_receipts"), 1);
  } finally {
    closeTestDatabase(testDatabase);
  }
});

test("an abruptly killed writer leaves no partial object or event on reopen", async () => {
  const testDatabase = await openTestDatabase();
  const hash = `sha256:${"0".repeat(64)}`;
  try {
    testDatabase.database.close();
    const script = `
      const { DatabaseSync } = require('node:sqlite');
      const db = new DatabaseSync(${JSON.stringify(testDatabase.path)});
      db.exec('PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; BEGIN IMMEDIATE');
      db.prepare('INSERT INTO gogoke_stream_heads(domain_id,stream_id,counter) VALUES (?,?,?)').run('crash-domain','crash-stream','0');
      db.prepare('INSERT INTO gogoke_objects(domain_id,object_type,object_id,object_version,canonical_json,content_hash,created_at) VALUES (?,?,?,?,?,?,?)').run('crash-domain','RuntimeInstance','["crash-object"]','["1"]',Buffer.from('{}'),${JSON.stringify(hash)},'2026-09-20T12:00:00.000Z');
      db.prepare('INSERT INTO gogoke_events(domain_id,event_id,stream_id,stream_counter,event_type,occurred_at,object_type,object_id,object_version,canonical_json,content_hash) VALUES (?,?,?,?,?,?,?,?,?,?,?)').run('crash-domain','crash-event','crash-stream','0','crash','2026-09-20T12:00:00.000Z','RuntimeInstance','["crash-object"]','["1"]',Buffer.from('{}'),${JSON.stringify(hash)});
      process.kill(process.pid, 'SIGKILL');
    `;
    const child = NodeChildProcess.spawnSync(process.execPath, ["-e", script], {
      timeout: 10_000,
    });
    assert.notEqual(child.status, 0, "crash probe must not exit successfully");

    const reopened = new NodeSqlite.DatabaseSync(testDatabase.path);
    try {
      assert.equal(
        readStringField(reopened.prepare("PRAGMA quick_check(1)").get(), "quick_check"),
        "ok",
      );
      assert.equal(count(reopened, "gogoke_objects"), 0);
      assert.equal(count(reopened, "gogoke_events"), 0);
      assert.equal(count(reopened, "gogoke_receipts"), 0);
    } finally {
      reopened.close();
    }
  } finally {
    NodeFS.rmSync(testDatabase.directory, { recursive: true, force: true });
  }
});
