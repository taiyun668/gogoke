export const CORE_MIGRATION_IDENTITY = Object.freeze({
  namespace: "gogoke.core",
  version: 1,
  name: "GogokeCoreRecords",
} as const);

const CORE_SCHEMA_VERSION = CORE_MIGRATION_IDENTITY.version;

const canonicalCounterCheck = (column: string) => `
  length(${column}) BETWEEN 1 AND 20
  AND ${column} NOT GLOB '*[^0-9]*'
  AND (${column} = '0' OR substr(${column}, 1, 1) <> '0')
`;

/**
 * The surrounding T3 migration runner owns version bookkeeping. These
 * statements intentionally create no journal and never inspect a legacy
 * Gogoke writer. Every table carries its schema version so an incompatible
 * pre-existing shape is observable instead of being normalized in place.
 */
const CORE_MIGRATION_STATEMENTS: ReadonlyArray<string> = Object.freeze([
  `
      CREATE TABLE IF NOT EXISTS gogoke_objects (
        schema_version INTEGER NOT NULL DEFAULT ${CORE_SCHEMA_VERSION}
          CHECK (schema_version = ${CORE_SCHEMA_VERSION}),
        domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
        object_type TEXT NOT NULL CHECK (length(object_type) > 0),
        object_id TEXT NOT NULL CHECK (length(object_id) > 0),
        object_version TEXT NOT NULL CHECK (length(object_version) > 0),
        canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
        content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
        created_at TEXT NOT NULL CHECK (length(created_at) > 0),
        PRIMARY KEY (domain_id, object_type, object_id, object_version)
      ) STRICT
    `,
  `
      CREATE TABLE IF NOT EXISTS gogoke_native_identities (
        schema_version INTEGER NOT NULL DEFAULT ${CORE_SCHEMA_VERSION}
          CHECK (schema_version = ${CORE_SCHEMA_VERSION}),
        domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
        runtime_instance_id TEXT NOT NULL CHECK (length(runtime_instance_id) > 0),
        native_id TEXT NOT NULL CHECK (length(native_id) > 0),
        object_type TEXT NOT NULL,
        object_id TEXT NOT NULL,
        object_version TEXT NOT NULL,
        PRIMARY KEY (domain_id, runtime_instance_id, native_id),
        FOREIGN KEY (domain_id, object_type, object_id, object_version)
          REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
          ON DELETE RESTRICT ON UPDATE RESTRICT
      ) STRICT
    `,
  `
      CREATE TABLE IF NOT EXISTS gogoke_stream_heads (
        schema_version INTEGER NOT NULL DEFAULT ${CORE_SCHEMA_VERSION}
          CHECK (schema_version = ${CORE_SCHEMA_VERSION}),
        domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
        stream_id TEXT NOT NULL CHECK (length(stream_id) > 0),
        counter TEXT NOT NULL CHECK (${canonicalCounterCheck("counter")}),
        PRIMARY KEY (domain_id, stream_id)
      ) STRICT
    `,
  `
      CREATE TABLE IF NOT EXISTS gogoke_events (
        schema_version INTEGER NOT NULL DEFAULT ${CORE_SCHEMA_VERSION}
          CHECK (schema_version = ${CORE_SCHEMA_VERSION}),
        domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
        event_id TEXT NOT NULL CHECK (length(event_id) > 0),
        stream_id TEXT NOT NULL CHECK (length(stream_id) > 0),
        stream_counter TEXT NOT NULL CHECK (${canonicalCounterCheck("stream_counter")}),
        event_type TEXT NOT NULL CHECK (length(event_type) > 0),
        occurred_at TEXT NOT NULL CHECK (length(occurred_at) > 0),
        object_type TEXT NOT NULL,
        object_id TEXT NOT NULL,
        object_version TEXT NOT NULL,
        canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
        content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
        PRIMARY KEY (domain_id, event_id),
        UNIQUE (domain_id, stream_id, stream_counter),
        FOREIGN KEY (domain_id, stream_id)
          REFERENCES gogoke_stream_heads (domain_id, stream_id)
          ON DELETE RESTRICT ON UPDATE RESTRICT,
        FOREIGN KEY (domain_id, object_type, object_id, object_version)
          REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
          ON DELETE RESTRICT ON UPDATE RESTRICT
      ) STRICT
    `,
  `
      CREATE TABLE IF NOT EXISTS gogoke_receipts (
        schema_version INTEGER NOT NULL DEFAULT ${CORE_SCHEMA_VERSION}
          CHECK (schema_version = ${CORE_SCHEMA_VERSION}),
        domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
        receipt_id TEXT NOT NULL CHECK (length(receipt_id) > 0),
        operation_id TEXT NOT NULL CHECK (length(operation_id) > 0),
        event_id TEXT NOT NULL,
        object_type TEXT NOT NULL,
        object_id TEXT NOT NULL,
        object_version TEXT NOT NULL,
        receipt_type TEXT NOT NULL CHECK (length(receipt_type) > 0),
        recorded_at TEXT NOT NULL CHECK (length(recorded_at) > 0),
        operation_fingerprint TEXT NOT NULL CHECK (length(operation_fingerprint) = 71),
        canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
        content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
        PRIMARY KEY (domain_id, receipt_id),
        UNIQUE (domain_id, operation_id),
        FOREIGN KEY (domain_id, event_id)
          REFERENCES gogoke_events (domain_id, event_id)
          ON DELETE RESTRICT ON UPDATE RESTRICT,
        FOREIGN KEY (domain_id, object_type, object_id, object_version)
          REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
          ON DELETE RESTRICT ON UPDATE RESTRICT
      ) STRICT
    `,
  `
      CREATE INDEX IF NOT EXISTS idx_gogoke_events_stream
      ON gogoke_events (domain_id, stream_id, stream_counter)
    `,
  `
      CREATE INDEX IF NOT EXISTS idx_gogoke_receipts_event
      ON gogoke_receipts (domain_id, event_id)
    `,
]);

export const CORE_TABLE_COLUMNS = Object.freeze({
  gogoke_objects: Object.freeze([
    "schema_version",
    "domain_id",
    "object_type",
    "object_id",
    "object_version",
    "canonical_json",
    "content_hash",
    "created_at",
  ]),
  gogoke_native_identities: Object.freeze([
    "schema_version",
    "domain_id",
    "runtime_instance_id",
    "native_id",
    "object_type",
    "object_id",
    "object_version",
  ]),
  gogoke_stream_heads: Object.freeze(["schema_version", "domain_id", "stream_id", "counter"]),
  gogoke_events: Object.freeze([
    "schema_version",
    "domain_id",
    "event_id",
    "stream_id",
    "stream_counter",
    "event_type",
    "occurred_at",
    "object_type",
    "object_id",
    "object_version",
    "canonical_json",
    "content_hash",
  ]),
  gogoke_receipts: Object.freeze([
    "schema_version",
    "domain_id",
    "receipt_id",
    "operation_id",
    "event_id",
    "object_type",
    "object_id",
    "object_version",
    "receipt_type",
    "recorded_at",
    "operation_fingerprint",
    "canonical_json",
    "content_hash",
  ]),
});

export type CoreTableName = keyof typeof CORE_TABLE_COLUMNS;

const CORE_TABLE_DEFINITIONS: Readonly<Record<CoreTableName, string>> = Object.freeze({
  gogoke_objects: CORE_MIGRATION_STATEMENTS[0]!,
  gogoke_native_identities: CORE_MIGRATION_STATEMENTS[1]!,
  gogoke_stream_heads: CORE_MIGRATION_STATEMENTS[2]!,
  gogoke_events: CORE_MIGRATION_STATEMENTS[3]!,
  gogoke_receipts: CORE_MIGRATION_STATEMENTS[4]!,
});

const CORE_INDEX_DEFINITIONS = Object.freeze({
  idx_gogoke_events_stream: CORE_MIGRATION_STATEMENTS[5]!,
  idx_gogoke_receipts_event: CORE_MIGRATION_STATEMENTS[6]!,
});

export type CoreIndexName = keyof typeof CORE_INDEX_DEFINITIONS;

export const CORE_TABLE_NAMES = Object.freeze(
  Object.keys(CORE_TABLE_COLUMNS) as ReadonlyArray<CoreTableName>,
);
export const CORE_INDEX_NAMES = Object.freeze(
  Object.keys(CORE_INDEX_DEFINITIONS) as ReadonlyArray<CoreIndexName>,
);

export class CoreSchemaMismatchError extends Error {
  readonly code = "CORE_SCHEMA_MISMATCH" as const;
  readonly table: CoreTableName;
  readonly observedColumns: ReadonlyArray<string>;

  constructor(table: CoreTableName, observedColumns: ReadonlyArray<string>) {
    super(`CORE_SCHEMA_MISMATCH: ${table} has an incompatible shape`);
    this.name = "CoreSchemaMismatchError";
    this.table = table;
    this.observedColumns = observedColumns;
  }
}

/** Fail closed when CREATE IF NOT EXISTS encountered an incompatible table. */
export function assertCoreSchemaColumns(
  observed: Readonly<Record<CoreTableName, ReadonlyArray<string>>>,
): void {
  for (const [table, expected] of Object.entries(CORE_TABLE_COLUMNS) as ReadonlyArray<
    readonly [CoreTableName, ReadonlyArray<string>]
  >) {
    const actual = observed[table];
    if (
      actual.length !== expected.length ||
      actual.some((column, index) => column !== expected[index])
    ) {
      throw new CoreSchemaMismatchError(table, actual);
    }
  }
}

const normalizeDefinition = (sql: string): string =>
  sql
    .replace(/\bIF\s+NOT\s+EXISTS\b/giu, "")
    .replace(/\s+/gu, " ")
    .replace(/\s*([(),])\s*/gu, "$1")
    .replace(/;$/u, "")
    .trim()
    .toLowerCase();

export interface CoreSchemaSnapshot {
  readonly columns: Readonly<Record<CoreTableName, ReadonlyArray<string>>>;
  readonly tableDefinitions: Readonly<Record<CoreTableName, string>>;
  readonly indexDefinitions: Readonly<Record<CoreIndexName, string>>;
}

/**
 * Exact definitions are checked in addition to column names: an old table with
 * the same fields but missing a foreign key, CHECK, UNIQUE, or STRICT clause is
 * not accepted as the current schema.
 */
export function assertCoreSchema(snapshot: CoreSchemaSnapshot): void {
  assertCoreSchemaColumns(snapshot.columns);
  for (const [name, expected] of Object.entries(CORE_TABLE_DEFINITIONS) as ReadonlyArray<
    readonly [CoreTableName, string]
  >) {
    if (normalizeDefinition(snapshot.tableDefinitions[name]) !== normalizeDefinition(expected)) {
      throw new CoreSchemaMismatchError(name, snapshot.columns[name]);
    }
  }
  for (const [name, expected] of Object.entries(CORE_INDEX_DEFINITIONS) as ReadonlyArray<
    readonly [CoreIndexName, string]
  >) {
    if (normalizeDefinition(snapshot.indexDefinitions[name]) !== normalizeDefinition(expected)) {
      throw new Error(`CORE_SCHEMA_MISMATCH: ${name} has an incompatible definition`);
    }
  }
}

export interface CoreMigrationTransaction {
  readonly execute: (statement: string) => Promise<void>;
  readonly inspectSchema: () => Promise<CoreSchemaSnapshot>;
}

export interface CoreMigrationDriver {
  /**
   * Trusted integration boundary, not a TypeScript custody proof. The future
   * shared production adapter must retain native-pinned DB custody internally.
   * The authoritative runner may journal CORE_MIGRATION_IDENTITY only after
   * this callback succeeds and must rollback DDL plus journal on failure.
   * This preparatory core provides no production adapter.
   */
  readonly withNativePinnedMigration: <T>(
    identity: typeof CORE_MIGRATION_IDENTITY,
    operation: (transaction: CoreMigrationTransaction) => Promise<T>,
  ) => Promise<T>;
}

/** The only sufficient core-migration entry point. Raw DDL is intentionally private. */
export async function applyCoreMigration(
  driver: CoreMigrationDriver,
): Promise<typeof CORE_MIGRATION_IDENTITY> {
  return driver.withNativePinnedMigration(CORE_MIGRATION_IDENTITY, async (transaction) => {
    for (const statement of CORE_MIGRATION_STATEMENTS) await transaction.execute(statement);
    assertCoreSchema(await transaction.inspectSchema());
    return CORE_MIGRATION_IDENTITY;
  });
}
