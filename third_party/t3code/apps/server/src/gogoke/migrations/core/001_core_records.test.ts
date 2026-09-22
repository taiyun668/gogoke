import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeSqlite from "node:sqlite";
import * as NodeTest from "node:test";

import {
  applyCoreMigration,
  CORE_MIGRATION_IDENTITY,
  CORE_TABLE_COLUMNS,
  type CoreIndexName,
  type CoreMigrationDriver,
  type CoreMigrationTransaction,
  type CoreSchemaSnapshot,
  type CoreTableName,
} from "./001_core_records.ts";

const readStringField = (row: unknown, field: string): string => {
  if (typeof row !== "object" || row === null) throw new Error(`${field} row is not an object`);
  const value = Reflect.get(row, field);
  if (typeof value !== "string") throw new Error(`${field} is not a string`);
  return value;
};

const readNumberField = (row: unknown, field: string): number => {
  if (typeof row !== "object" || row === null) throw new Error(`${field} row is not an object`);
  const value = Reflect.get(row, field);
  if (typeof value !== "number") throw new Error(`${field} is not a number`);
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

function schemaSnapshot(database: NodeSqlite.DatabaseSync): CoreSchemaSnapshot {
  const columns = mapCoreTables((table) =>
    database
      .prepare(`PRAGMA table_info(${table})`)
      .all()
      .map((row) => readStringField(row, "name")),
  );
  const definition = (name: string, type: "table" | "index"): string => {
    const row = database
      .prepare("SELECT sql FROM sqlite_schema WHERE type = ? AND name = ?")
      .get(type, name);
    return row === undefined ? "" : readStringField(row, "sql");
  };
  return {
    columns,
    tableDefinitions: mapCoreTables((table) => definition(table, "table")),
    indexDefinitions: mapCoreIndexes((index) => definition(index, "index")),
  };
}

class ControlledTestMigrationDriver implements CoreMigrationDriver {
  readonly journaledIdentities = new Set<string>();
  readonly database: NodeSqlite.DatabaseSync;
  nativePinnedMigrationCalls = 0;

  constructor(database: NodeSqlite.DatabaseSync) {
    this.database = database;
  }

  async withNativePinnedMigration<T>(
    identity: typeof CORE_MIGRATION_IDENTITY,
    operation: (transaction: CoreMigrationTransaction) => Promise<T>,
  ): Promise<T> {
    this.nativePinnedMigrationCalls += 1;
    this.database.exec("BEGIN IMMEDIATE");
    try {
      const result = await operation({
        execute: async (statement) => this.database.exec(statement),
        inspectSchema: async () => schemaSnapshot(this.database),
      });
      this.database.exec("COMMIT");
      // This models the authoritative runner rule: no journal observation is
      // possible until create+inspection and commit have completed.
      this.journaledIdentities.add(`${identity.namespace}:${identity.version}`);
      return result;
    } catch (error) {
      this.database.exec("ROLLBACK");
      throw error;
    }
  }
}

async function withDatabase(
  run: (driver: ControlledTestMigrationDriver) => Promise<void>,
): Promise<void> {
  const directory = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "gogoke-core-migration-"));
  const database = new NodeSqlite.DatabaseSync(NodePath.join(directory, "state.sqlite"));
  try {
    database.exec("PRAGMA foreign_keys = ON");
    await run(new ControlledTestMigrationDriver(database));
  } finally {
    database.close();
    NodeFS.rmSync(directory, { recursive: true, force: true });
  }
}

NodeTest.test(
  "apply operation uses a namespaced identity, verifies schema, and remains idempotent",
  () =>
    withDatabase(async (driver) => {
      NodeAssert.deepEqual(CORE_MIGRATION_IDENTITY, {
        namespace: "gogoke.core",
        version: 1,
        name: "GogokeCoreRecords",
      });

      NodeAssert.equal(await applyCoreMigration(driver), CORE_MIGRATION_IDENTITY);
      NodeAssert.equal(await applyCoreMigration(driver), CORE_MIGRATION_IDENTITY);
      NodeAssert.equal(driver.nativePinnedMigrationCalls, 2);
      NodeAssert.deepEqual([...driver.journaledIdentities], ["gogoke.core:1"]);

      const journals = driver.database
        .prepare(
          `SELECT name FROM sqlite_schema
         WHERE type = 'table' AND lower(name) LIKE '%migration%'`,
        )
        .all();
      NodeAssert.deepEqual(journals, []);
    }),
);

NodeTest.test("same columns without exact constraints fail apply and preserve the original", () =>
  withDatabase(async (driver) => {
    const weakColumns = CORE_TABLE_COLUMNS.gogoke_objects
      .map((column) => `${column} TEXT`)
      .join(", ");
    driver.database.exec(`CREATE TABLE gogoke_objects (${weakColumns})`);
    driver.database
      .prepare(
        `INSERT INTO gogoke_objects VALUES (${CORE_TABLE_COLUMNS.gogoke_objects
          .map(() => "?")
          .join(",")})`,
      )
      .run(...CORE_TABLE_COLUMNS.gogoke_objects.map(() => "preserve-me"));

    await NodeAssert.rejects(
      applyCoreMigration(driver),
      (error: unknown) =>
        error instanceof Error && error.message.startsWith("CORE_SCHEMA_MISMATCH:"),
    );
    NodeAssert.equal(driver.journaledIdentities.size, 0);
    NodeAssert.equal(
      readNumberField(
        driver.database.prepare("SELECT COUNT(*) AS count FROM gogoke_objects").get(),
        "count",
      ),
      1,
    );
    NodeAssert.equal(
      readNumberField(
        driver.database
          .prepare("SELECT COUNT(*) AS count FROM sqlite_schema WHERE name = 'gogoke_events'")
          .get(),
        "count",
      ),
      0,
    );
  }),
);

NodeTest.test("migration core delegates custody to the trusted adapter capability", () =>
  withDatabase(async (driver) => {
    NodeAssert.equal("pinnedDatabaseIdentity" in driver, false);
    NodeAssert.equal(driver.nativePinnedMigrationCalls, 0);
    await applyCoreMigration(driver);
    NodeAssert.equal(driver.nativePinnedMigrationCalls, 1);
  }),
);
