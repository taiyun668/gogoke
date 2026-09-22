import * as NodeAssert from "node:assert/strict";
import * as NodeCrypto from "node:crypto";
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeSqlite from "node:sqlite";
import * as NodeTest from "node:test";

import { DatabasePreflightError, preflightDatabaseFile } from "./databasePreflight.ts";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

const digest = (path: string): string =>
  NodeCrypto.createHash("sha256").update(NodeFS.readFileSync(path)).digest("hex");

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

function withTempDirectory(run: (directory: string) => Promise<void>): Promise<void> {
  const directory = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "gogoke-db-preflight-"));
  return run(directory).finally(() => NodeFS.rmSync(directory, { recursive: true, force: true }));
}

const rejectsWith =
  (code: DatabasePreflightError["code"]) =>
  (error: unknown): boolean =>
    error instanceof DatabasePreflightError && error.code === code;

test("missing database is reported without creating an empty replacement", () =>
  withTempDirectory(async (directory) => {
    const path = NodePath.join(directory, "missing.sqlite");
    assert.deepEqual(await preflightDatabaseFile(path), {
      kind: "NEW_DATABASE",
      path,
      authorizesWritableOpen: false,
    });
    assert.throws(() => NodeFS.readFileSync(path), { code: "ENOENT" });
  }));

test("valid database is reported without Node opening SQLite", () =>
  withTempDirectory(async (directory) => {
    const path = NodePath.join(directory, "valid.sqlite");
    const database = new NodeSqlite.DatabaseSync(path);
    database.exec("CREATE TABLE payload(id INTEGER PRIMARY KEY, value TEXT NOT NULL)");
    database.prepare("INSERT INTO payload(value) VALUES (?)").run("durable");
    database.close();

    const before = digest(path);
    const result = await preflightDatabaseFile(path);
    assert.equal(result.kind, "EXISTING_SQLITE");
    assert.equal(result.authorizesWritableOpen, false);
    assert.equal(digest(path), before);
  }));

test("bad header and incomplete tail are rejected without modifying their bytes", async (t) => {
  await t.test("bad header", () =>
    withTempDirectory(async (directory) => {
      const path = NodePath.join(directory, "bad-header.sqlite");
      NodeFS.writeFileSync(path, Buffer.alloc(4096, 0x5a));
      const before = digest(path);
      await assert.rejects(preflightDatabaseFile(path), rejectsWith("DATABASE_HEADER_INVALID"));
      assert.equal(digest(path), before);
    }),
  );

  await t.test("incomplete tail", () =>
    withTempDirectory(async (directory) => {
      const path = NodePath.join(directory, "truncated.sqlite");
      const database = new NodeSqlite.DatabaseSync(path);
      database.exec("CREATE TABLE payload(id INTEGER PRIMARY KEY)");
      database.close();
      NodeFS.truncateSync(path, NodeFS.readFileSync(path).byteLength - 1);
      const before = digest(path);
      await assert.rejects(preflightDatabaseFile(path), rejectsWith("DATABASE_TAIL_INCOMPLETE"));
      assert.equal(digest(path), before);
    }),
  );
});

test("page corruption is not diagnosed by Node SQLite; original bytes stay unchanged", () =>
  withTempDirectory(async (directory) => {
    const path = NodePath.join(directory, "corrupt.sqlite");
    const database = new NodeSqlite.DatabaseSync(path);
    database.exec("PRAGMA page_size = 512; VACUUM; CREATE TABLE payload(value TEXT NOT NULL)");
    const insert = database.prepare("INSERT INTO payload(value) VALUES (?)");
    for (let index = 0; index < 200; index += 1) insert.run("x".repeat(400));
    database.close();

    const descriptor = NodeFS.openSync(path, "r+");
    try {
      NodeFS.writeSync(descriptor, Buffer.alloc(16), 0, 16, 512);
    } finally {
      NodeFS.closeSync(descriptor);
    }
    const before = digest(path);
    const result = await preflightDatabaseFile(path);
    assert.equal(result.kind, "EXISTING_SQLITE");
    assert.equal(result.authorizesWritableOpen, false);
    assert.equal(digest(path), before);
  }));

test("an incomplete WAL frame is rejected and the sidecar remains byte-identical", () =>
  withTempDirectory(async (directory) => {
    const path = NodePath.join(directory, "incomplete-wal.sqlite");
    const database = new NodeSqlite.DatabaseSync(path);
    database.exec("PRAGMA journal_mode = WAL; CREATE TABLE payload(id INTEGER PRIMARY KEY)");
    const pageSize = readNumberField(database.prepare("PRAGMA page_size").get(), "page_size");
    database.close();

    const walPath = `${path}-wal`;
    const header = Buffer.alloc(32);
    header.writeUInt32BE(0x377f0682, 0);
    header.writeUInt32BE(3_007_000, 4);
    header.writeUInt32BE(pageSize, 8);
    NodeFS.writeFileSync(walPath, Buffer.concat([header, Buffer.from([0xff])]));
    const before = digest(walPath);

    await assert.rejects(preflightDatabaseFile(path), rejectsWith("DATABASE_TAIL_INCOMPLETE"));
    assert.equal(digest(walPath), before);
  }));

function createWalOnlyRow(path: string): NodeSqlite.DatabaseSync {
  const database = new NodeSqlite.DatabaseSync(path);
  database.exec(
    `PRAGMA journal_mode = WAL;
     PRAGMA wal_autocheckpoint = 0;
     CREATE TABLE payload(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
     PRAGMA wal_checkpoint(TRUNCATE);`,
  );
  database.prepare("INSERT INTO payload(value) VALUES (?)").run("wal-only");
  return database;
}

test("valid raw WAL checksums expose a row that exists only in the WAL", () =>
  withTempDirectory(async (directory) => {
    const path = NodePath.join(directory, "wal-only.sqlite");
    const database = createWalOnlyRow(path);
    try {
      const mainOnly = NodePath.join(directory, "main-only.sqlite");
      NodeFS.copyFileSync(path, mainOnly);
      const mainDatabase = new NodeSqlite.DatabaseSync(mainOnly, { readOnly: true });
      try {
        assert.equal(
          readNumberField(
            mainDatabase.prepare("SELECT COUNT(*) AS count FROM payload").get(),
            "count",
          ),
          0,
        );
      } finally {
        mainDatabase.close();
      }

      const result = await preflightDatabaseFile(path);
      assert.equal(result.kind, "EXISTING_SQLITE");
      assert.equal(result.authorizesWritableOpen, false);
      const reader = new NodeSqlite.DatabaseSync(path, { readOnly: true });
      try {
        assert.equal(
          readStringField(reader.prepare("SELECT value FROM payload").get(), "value"),
          "wal-only",
        );
      } finally {
        reader.close();
      }
    } finally {
      database.close();
    }
  }));

test("bad WAL frame salt and checksum fail closed without changing bytes", async (t) => {
  await t.test("salt", () =>
    withTempDirectory(async (directory) => {
      const source = NodePath.join(directory, "source.sqlite");
      const database = createWalOnlyRow(source);
      try {
        const target = NodePath.join(directory, "bad-salt.sqlite");
        NodeFS.copyFileSync(source, target);
        NodeFS.copyFileSync(`${source}-wal`, `${target}-wal`);
        const descriptor = NodeFS.openSync(`${target}-wal`, "r+");
        try {
          const saltByte = Buffer.alloc(1);
          assert.equal(NodeFS.readFileSync(`${target}-wal`).byteLength > 40, true);
          saltByte[0] = NodeFS.readFileSync(`${target}-wal`)[40]! ^ 0xff;
          NodeFS.writeSync(descriptor, saltByte, 0, 1, 40);
        } finally {
          NodeFS.closeSync(descriptor);
        }
        const before = digest(`${target}-wal`);
        await assert.rejects(
          preflightDatabaseFile(target),
          rejectsWith("DATABASE_INTEGRITY_FAILED"),
        );
        assert.equal(digest(`${target}-wal`), before);
      } finally {
        database.close();
      }
    }),
  );

  await t.test("checksum", () =>
    withTempDirectory(async (directory) => {
      const source = NodePath.join(directory, "source.sqlite");
      const database = createWalOnlyRow(source);
      try {
        const target = NodePath.join(directory, "bad-checksum.sqlite");
        NodeFS.copyFileSync(source, target);
        NodeFS.copyFileSync(`${source}-wal`, `${target}-wal`);
        const descriptor = NodeFS.openSync(`${target}-wal`, "r+");
        try {
          const checksumByte = Buffer.from([NodeFS.readFileSync(`${target}-wal`)[48]! ^ 0xff]);
          NodeFS.writeSync(descriptor, checksumByte, 0, 1, 48);
        } finally {
          NodeFS.closeSync(descriptor);
        }
        const before = digest(`${target}-wal`);
        await assert.rejects(
          preflightDatabaseFile(target),
          rejectsWith("DATABASE_INTEGRITY_FAILED"),
        );
        assert.equal(digest(`${target}-wal`), before);
      } finally {
        database.close();
      }
    }),
  );
});

test("production preflight source never constructs node:sqlite", () => {
  const source = NodeFS.readFileSync(new URL("./databasePreflight.ts", import.meta.url), "utf8");
  assert.equal(source.includes("node:sqlite"), false);
  assert.equal(source.includes("DatabaseSync"), false);
  assert.equal(source.includes("assertQuickCheck"), false);
});
