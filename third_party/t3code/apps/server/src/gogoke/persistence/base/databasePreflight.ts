import * as NodeFSP from "node:fs/promises";
import * as NodePath from "node:path";

const SQLITE_HEADER = Buffer.from("SQLite format 3\0", "ascii");
const HEADER_BYTES = 100;
const WAL_HEADER_BYTES = 32;
const WAL_FRAME_HEADER_BYTES = 24;
const WAL_MAGIC_CHECKSUM_LITTLE_ENDIAN = 0x377f0682;
const WAL_MAGIC_CHECKSUM_BIG_ENDIAN = 0x377f0683;
const WAL_FORMAT_VERSION = 3_007_000;

export type DatabasePreflightResult =
  | {
      readonly kind: "NEW_DATABASE";
      readonly path: string;
      readonly authorizesWritableOpen: false;
    }
  | {
      readonly kind: "EXISTING_SQLITE";
      readonly path: string;
      readonly pageSize: number;
      readonly pageCount: bigint;
      readonly size: bigint;
      readonly authorizesWritableOpen: false;
    };

export type DatabasePreflightErrorCode =
  | "DATABASE_PATH_NOT_ABSOLUTE"
  | "DATABASE_PATH_NOT_FILE"
  | "DATABASE_SYMLINK_REFUSED"
  | "DATABASE_HEADER_INVALID"
  | "DATABASE_TAIL_INCOMPLETE"
  | "DATABASE_INTEGRITY_FAILED"
  | "DATABASE_CHANGED_DURING_PREFLIGHT"
  | "DATABASE_READ_FAILED";

export class DatabasePreflightError extends Error {
  override readonly name = "DatabasePreflightError";
  readonly code: DatabasePreflightErrorCode;
  readonly databasePath: string;
  override readonly cause: unknown;

  constructor(
    code: DatabasePreflightErrorCode,
    databasePath: string,
    detail: string,
    cause?: unknown,
  ) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.databasePath = databasePath;
    this.cause = cause;
  }
}

const isMissing = (error: unknown): boolean =>
  typeof error === "object" && error !== null && Reflect.get(error, "code") === "ENOENT";

const readU32 = (header: Buffer, offset: number): number => header.readUInt32BE(offset);

function decodePageSize(header: Buffer, databasePath: string): number {
  const encoded = header.readUInt16BE(16);
  const pageSize = encoded === 1 ? 65_536 : encoded;
  if (pageSize < 512 || pageSize > 65_536 || (pageSize & (pageSize - 1)) !== 0) {
    throw new DatabasePreflightError(
      "DATABASE_HEADER_INVALID",
      databasePath,
      `invalid SQLite page size ${pageSize}`,
    );
  }
  return pageSize;
}

function assertHeader(header: Buffer, databasePath: string): number {
  if (
    header.length !== HEADER_BYTES ||
    !header.subarray(0, SQLITE_HEADER.length).equals(SQLITE_HEADER)
  ) {
    throw new DatabasePreflightError(
      "DATABASE_HEADER_INVALID",
      databasePath,
      "SQLite header signature is missing",
    );
  }
  const pageSize = decodePageSize(header, databasePath);
  const writeVersion = header[18];
  const readVersion = header[19];
  const reservedBytes = header[20] ?? pageSize;
  const schemaFormat = readU32(header, 44);
  if (
    (writeVersion !== 1 && writeVersion !== 2) ||
    (readVersion !== 1 && readVersion !== 2) ||
    reservedBytes >= pageSize ||
    header[21] !== 64 ||
    header[22] !== 32 ||
    header[23] !== 32 ||
    schemaFormat < 1 ||
    schemaFormat > 4
  ) {
    throw new DatabasePreflightError(
      "DATABASE_HEADER_INVALID",
      databasePath,
      "SQLite header fields are inconsistent",
    );
  }
  return pageSize;
}

interface WalChecksum {
  readonly first: number;
  readonly second: number;
}

const readChecksumWord = (buffer: Buffer, offset: number, littleEndian: boolean): number =>
  littleEndian ? buffer.readUInt32LE(offset) : buffer.readUInt32BE(offset);

function updateWalChecksum(buffer: Buffer, littleEndian: boolean, prior: WalChecksum): WalChecksum {
  if (buffer.byteLength % 8 !== 0) {
    throw new Error("WAL checksum input must contain complete pairs of uint32 words");
  }
  let first = prior.first;
  let second = prior.second;
  for (let offset = 0; offset < buffer.byteLength; offset += 8) {
    const left = readChecksumWord(buffer, offset, littleEndian);
    const right = readChecksumWord(buffer, offset + 4, littleEndian);
    first = (first + left + second) >>> 0;
    second = (second + right + first) >>> 0;
  }
  return { first, second };
}

async function assertWalTail(databasePath: string, pageSize: number): Promise<void> {
  const walPath = `${databasePath}-wal`;
  let stats;
  try {
    stats = await NodeFSP.lstat(walPath, { bigint: true });
  } catch (error) {
    if (isMissing(error)) return;
    throw new DatabasePreflightError(
      "DATABASE_READ_FAILED",
      walPath,
      "WAL metadata could not be read",
      error,
    );
  }
  if (stats.isSymbolicLink()) {
    throw new DatabasePreflightError(
      "DATABASE_SYMLINK_REFUSED",
      walPath,
      "WAL file symlinks are not admitted",
    );
  }
  if (!stats.isFile()) {
    throw new DatabasePreflightError(
      "DATABASE_PATH_NOT_FILE",
      walPath,
      "existing WAL path is not a regular file",
    );
  }
  if (stats.size === 0n) return;
  if (stats.size < BigInt(WAL_HEADER_BYTES)) {
    throw new DatabasePreflightError(
      "DATABASE_TAIL_INCOMPLETE",
      walPath,
      "WAL ends before its header is complete",
    );
  }

  const frameSize = WAL_FRAME_HEADER_BYTES + pageSize;
  if ((stats.size - BigInt(WAL_HEADER_BYTES)) % BigInt(frameSize) !== 0n) {
    throw new DatabasePreflightError(
      "DATABASE_TAIL_INCOMPLETE",
      walPath,
      "WAL ends in an incomplete frame",
    );
  }

  const handle = await NodeFSP.open(walPath, "r");
  try {
    const header = Buffer.alloc(WAL_HEADER_BYTES);
    const read = await handle.read(header, 0, WAL_HEADER_BYTES, 0);
    if (read.bytesRead !== WAL_HEADER_BYTES) {
      throw new DatabasePreflightError(
        "DATABASE_TAIL_INCOMPLETE",
        walPath,
        "WAL header could not be read completely",
      );
    }
    const magic = header.readUInt32BE(0);
    const checksumLittleEndian = magic === WAL_MAGIC_CHECKSUM_LITTLE_ENDIAN;
    const validMagic = checksumLittleEndian || magic === WAL_MAGIC_CHECKSUM_BIG_ENDIAN;
    const walPageSize = header.readUInt32BE(8);
    if (!validMagic || header.readUInt32BE(4) !== WAL_FORMAT_VERSION || walPageSize !== pageSize) {
      throw new DatabasePreflightError(
        "DATABASE_HEADER_INVALID",
        walPath,
        "WAL format version or page size does not match the SQLite database",
      );
    }

    const saltOne = header.readUInt32BE(16);
    const saltTwo = header.readUInt32BE(20);
    let checksum = updateWalChecksum(header.subarray(0, 24), checksumLittleEndian, {
      first: 0,
      second: 0,
    });
    if (checksum.first !== header.readUInt32BE(24) || checksum.second !== header.readUInt32BE(28)) {
      throw new DatabasePreflightError(
        "DATABASE_INTEGRITY_FAILED",
        walPath,
        "WAL header checksum is invalid",
      );
    }

    const frameCount = Number((stats.size - BigInt(WAL_HEADER_BYTES)) / BigInt(frameSize));
    const frame = Buffer.alloc(frameSize);
    for (let index = 0; index < frameCount; index += 1) {
      const position = WAL_HEADER_BYTES + index * frameSize;
      const frameRead = await handle.read(frame, 0, frameSize, position);
      if (frameRead.bytesRead !== frameSize) {
        throw new DatabasePreflightError(
          "DATABASE_TAIL_INCOMPLETE",
          walPath,
          `WAL frame ${index} could not be read completely`,
        );
      }
      if (
        frame.readUInt32BE(0) === 0 ||
        frame.readUInt32BE(8) !== saltOne ||
        frame.readUInt32BE(12) !== saltTwo
      ) {
        throw new DatabasePreflightError(
          "DATABASE_INTEGRITY_FAILED",
          walPath,
          `WAL frame ${index} has an invalid page number or salt`,
        );
      }
      checksum = updateWalChecksum(frame.subarray(0, 8), checksumLittleEndian, checksum);
      checksum = updateWalChecksum(
        frame.subarray(WAL_FRAME_HEADER_BYTES),
        checksumLittleEndian,
        checksum,
      );
      if (checksum.first !== frame.readUInt32BE(16) || checksum.second !== frame.readUInt32BE(20)) {
        throw new DatabasePreflightError(
          "DATABASE_INTEGRITY_FAILED",
          walPath,
          `WAL frame ${index} checksum is invalid`,
        );
      }
    }
  } finally {
    await handle.close();
  }
}

interface StableFileIdentity {
  readonly dev: bigint;
  readonly ino: bigint;
  readonly size: bigint;
  readonly mtimeNs: bigint;
}

const sameFile = (left: StableFileIdentity, right: StableFileIdentity): boolean =>
  left.dev === right.dev &&
  left.ino === right.ino &&
  left.size === right.size &&
  left.mtimeNs === right.mtimeNs;

/**
 * Diagnose an explicit database path without creating, repairing, replacing,
 * or migrating it. This path/stat inspection is never authority for writable
 * open: a later replacement can only be excluded by native pinned-file custody.
 */
export async function preflightDatabaseFile(
  databasePath: string,
): Promise<DatabasePreflightResult> {
  if (!NodePath.isAbsolute(databasePath)) {
    throw new DatabasePreflightError(
      "DATABASE_PATH_NOT_ABSOLUTE",
      databasePath,
      "an explicit absolute database path is required",
    );
  }

  let pathStats;
  try {
    pathStats = await NodeFSP.lstat(databasePath, { bigint: true });
  } catch (error) {
    if (isMissing(error)) {
      return { kind: "NEW_DATABASE", path: databasePath, authorizesWritableOpen: false };
    }
    throw new DatabasePreflightError(
      "DATABASE_READ_FAILED",
      databasePath,
      "database metadata could not be read",
      error,
    );
  }
  if (pathStats.isSymbolicLink()) {
    throw new DatabasePreflightError(
      "DATABASE_SYMLINK_REFUSED",
      databasePath,
      "database file symlinks are not admitted",
    );
  }
  if (!pathStats.isFile()) {
    throw new DatabasePreflightError(
      "DATABASE_PATH_NOT_FILE",
      databasePath,
      "existing database path is not a regular file",
    );
  }

  const handle = await NodeFSP.open(databasePath, "r").catch((error: unknown) => {
    throw new DatabasePreflightError(
      "DATABASE_READ_FAILED",
      databasePath,
      "database could not be opened read-only",
      error,
    );
  });
  let before: StableFileIdentity;
  let header: Buffer;
  try {
    const stats = await handle.stat({ bigint: true });
    before = { dev: stats.dev, ino: stats.ino, size: stats.size, mtimeNs: stats.mtimeNs };
    header = Buffer.alloc(HEADER_BYTES);
    const read = await handle.read(header, 0, HEADER_BYTES, 0);
    if (read.bytesRead !== HEADER_BYTES) {
      throw new DatabasePreflightError(
        "DATABASE_HEADER_INVALID",
        databasePath,
        `existing file has only ${read.bytesRead} header bytes`,
      );
    }
  } finally {
    await handle.close();
  }

  const pageSize = assertHeader(header, databasePath);
  const pageSizeBigInt = BigInt(pageSize);
  if (before.size < pageSizeBigInt || before.size % pageSizeBigInt !== 0n) {
    throw new DatabasePreflightError(
      "DATABASE_TAIL_INCOMPLETE",
      databasePath,
      `file size ${before.size} is not a complete sequence of ${pageSize}-byte pages`,
    );
  }

  await assertWalTail(databasePath, pageSize);
  const afterStats = await NodeFSP.lstat(databasePath, { bigint: true });
  const after: StableFileIdentity = {
    dev: afterStats.dev,
    ino: afterStats.ino,
    size: afterStats.size,
    mtimeNs: afterStats.mtimeNs,
  };
  if (!sameFile(before, after)) {
    throw new DatabasePreflightError(
      "DATABASE_CHANGED_DURING_PREFLIGHT",
      databasePath,
      "database identity or bytes changed during read-only validation",
    );
  }

  return {
    kind: "EXISTING_SQLITE",
    path: databasePath,
    pageSize,
    pageCount: before.size / pageSizeBigInt,
    size: before.size,
    authorizesWritableOpen: false,
  };
}
