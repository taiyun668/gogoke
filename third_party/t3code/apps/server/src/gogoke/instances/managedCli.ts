/** Fixed official Windows x64 CLI staging. This module handles data only:
 * it never runs npm, a package script, installer, or a downloaded executable.
 * The native Owner issuer must own activation and remeasure the image bytes.
 */
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { lstat, mkdir, mkdtemp, open, realpath, rename, rm, stat } from "node:fs/promises";
import { join, resolve, sep } from "node:path";
import { pipeline } from "node:stream/promises";
import { createGunzip } from "node:zlib";
import { FIXED_OFFICIAL_CATALOG } from "./fixedOfficialCatalog.ts";

export type FixedDriver = (typeof FIXED_OFFICIAL_CATALOG)[number]["driver"];
export type StagedCli = Readonly<{
  driver: FixedDriver;
  version: string;
  stagePath: string;
  imagePath: string;
  archiveSha256: string;
  imageSha256: string;
}>;
export type StageProgress = (phase: "downloading" | "verifying" | "extracting", bytes: number) => void;

const MAX_ARCHIVE_BYTES = 250 * 1024 * 1024;
const MAX_UNPACKED_BYTES = 600 * 1024 * 1024;
const HEADER_BYTES = 512;

export class ManagedCliError extends Error {
  readonly code: string;
  constructor(code: string, message: string) { super(message); this.code = code; }
}

export function fixedOfficialCli(driver: string) {
  return FIXED_OFFICIAL_CATALOG.find((entry) => entry.driver === driver) ?? null;
}

/** Official registry metadata is a notice only; this return value can never
 * become an installable version without a separately pinned archive/image. */
export async function checkUnverifiedOfficialVersion(driver: FixedDriver): Promise<string | null> {
  const packageName = ({ codex: "@openai/codex", claude: "@anthropic-ai/claude-code",
    opencode: "opencode-ai", grok: null } as const)[driver];
  if (!packageName) return null;
  const url = `https://registry.npmjs.org/${encodeURIComponent(packageName)}/latest`;
  const response = await fetch(url, { redirect: "manual", cache: "no-store" });
  if (!response.ok || response.url !== url) {
    throw new ManagedCliError("UPDATE_CHECK", `official metadata ${response.status} ${response.statusText}`);
  }
  const length = Number(response.headers.get("content-length") ?? 0);
  if (length > 256 * 1024) throw new ManagedCliError("UPDATE_CHECK", "metadata too large");
  const raw = await response.text();
  if (raw.length > 256 * 1024) throw new ManagedCliError("UPDATE_CHECK", "metadata too large");
  const metadata: unknown = JSON.parse(raw);
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    throw new ManagedCliError("UPDATE_CHECK", "invalid official metadata");
  }
  const fields = metadata as Record<string, unknown>;
  if (fields.name !== packageName || typeof fields.version !== "string" ||
      !/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/.test(fields.version)) {
    throw new ManagedCliError("UPDATE_CHECK", "invalid official version");
  }
  return fields.version;
}

async function plainDirectory(path: string): Promise<void> {
  const item = await lstat(path);
  if (!item.isDirectory() || item.isSymbolicLink()) {
    throw new ManagedCliError("ROOT_IDENTITY", `not a plain directory: ${path}`);
  }
  if (await realpath(path) !== resolve(path)) {
    throw new ManagedCliError("ROOT_IDENTITY", `directory resolves elsewhere: ${path}`);
  }
}

async function digestFile(path: string): Promise<string> {
  const digest = createHash("sha256");
  for await (const bytes of createReadStream(path)) digest.update(bytes);
  return digest.digest("hex");
}

async function downloadPinned(url: string, destination: string, progress?: StageProgress): Promise<void> {
  // The URL is always chosen from the compiled catalog, never a request.
  const expected = new URL(url);
  if (expected.protocol !== "https:" || !["registry.npmjs.org", "x.ai"].includes(expected.hostname)) {
    throw new ManagedCliError("SOURCE", "catalog source is not allowed");
  }
  const response = await fetch(url, { redirect: "manual", cache: "no-store" });
  if (!response.ok || response.url !== url || !response.body) {
    throw new ManagedCliError("DOWNLOAD", `official response ${response.status} ${response.statusText}`);
  }
  let received = 0;
  const output = createWriteStream(destination, { flags: "wx" });
  try {
    for await (const chunk of response.body) {
      const bytes = Buffer.from(chunk);
      received += bytes.length;
      if (received > MAX_ARCHIVE_BYTES) throw new ManagedCliError("SIZE", "archive exceeds fixed limit");
      if (!output.write(bytes)) await new Promise<void>((done) => output.once("drain", done));
      progress?.("downloading", received);
    }
    await new Promise<void>((done, reject) => output.end((error?: Error | null) => error ? reject(error) : done()));
  } catch (error) {
    output.destroy();
    throw error;
  }
}

function safeMember(name: string): boolean {
  return name.startsWith("package/") && !name.includes("\\") && !name.includes(":") &&
    !name.startsWith("/") && name.split("/").every((part) => part !== "" && part !== "." && part !== "..");
}

function tarString(header: Buffer, start: number, length: number): string {
  const field = header.subarray(start, start + length);
  const zero = field.indexOf(0);
  return field.subarray(0, zero < 0 ? field.length : zero).toString("utf8");
}

function tarOctal(header: Buffer, start: number, length: number): number {
  const value = tarString(header, start, length).trim();
  if (!/^[0-7]+$/.test(value)) throw new ManagedCliError("TAR", "invalid tar number");
  return Number.parseInt(value, 8);
}

function tarHeader(header: Buffer): { name: string; size: number } {
  if (header.length !== HEADER_BYTES) throw new ManagedCliError("TAR", "truncated header");
  const expected = tarOctal(header, 148, 8);
  const checksum = header.reduce((sum, byte, index) =>
    sum + (index >= 148 && index < 156 ? 32 : byte), 0);
  if (checksum !== expected) throw new ManagedCliError("TAR", "header checksum mismatch");
  const prefix = tarString(header, 345, 155);
  const name = `${prefix ? `${prefix}/` : ""}${tarString(header, 0, 100)}`;
  const type = header[156];
  if (type !== 0 && type !== 48) throw new ManagedCliError("TAR", "link or non-file archive member");
  const size = tarOctal(header, 124, 12);
  if (!Number.isSafeInteger(size) || size > MAX_UNPACKED_BYTES) {
    throw new ManagedCliError("SIZE", "invalid member size");
  }
  return { name, size };
}

async function extractAllowlistedTar(tarPath: string, destination: string,
  files: Readonly<Record<string, number>>, progress?: StageProgress): Promise<void> {
  const input = await open(tarPath, "r");
  const seen = new Set<string>();
  try {
    const length = (await input.stat()).size;
    let position = 0;
    let total = 0;
    while (position + HEADER_BYTES <= length) {
      const header = Buffer.alloc(HEADER_BYTES);
      await input.read(header, 0, HEADER_BYTES, position);
      position += HEADER_BYTES;
      if (header.every((byte) => byte === 0)) {
        // All trailing blocks must be zero, including archive padding.
        const tail = Buffer.alloc(64 * 1024);
        while (position < length) {
          const read = await input.read(tail, 0, Math.min(tail.length, length - position), position);
          if (read.bytesRead === 0 || tail.subarray(0, read.bytesRead).some((byte) => byte !== 0)) {
            throw new ManagedCliError("TAR", "nonzero tar trailer");
          }
          position += read.bytesRead;
        }
        break;
      }
      const member = tarHeader(header);
      if (!safeMember(member.name) || !(member.name in files) || files[member.name] !== member.size ||
          seen.has(member.name)) {
        throw new ManagedCliError("TAR", `unexpected archive member: ${member.name}`);
      }
      seen.add(member.name);
      total += member.size;
      if (total > MAX_UNPACKED_BYTES || position + member.size > length) {
        throw new ManagedCliError("SIZE", "archive member exceeds bound");
      }
      const target = join(destination, ...member.name.split("/"));
      if (!target.startsWith(`${destination}${sep}`)) throw new ManagedCliError("TAR", "path escapes staging");
      await mkdir(resolve(target, ".."), { recursive: true });
      if (member.size === 0) {
        const empty = await open(target, "wx");
        await empty.close();
      } else {
        await pipeline(createReadStream(tarPath, { start: position, end: position + member.size - 1 }),
          createWriteStream(target, { flags: "wx" }));
      }
      progress?.("extracting", total);
      position += Math.ceil(member.size / HEADER_BYTES) * HEADER_BYTES;
    }
    if (position !== length || seen.size !== Object.keys(files).length) {
      throw new ManagedCliError("TAR", "archive member inventory incomplete");
    }
  } finally { await input.close(); }
}

/** The returned path is staging only. No CLI state is READY until native
 * activation rechecks the actual file and H proves it can launch. */
export async function stageFixedOfficialCli(root: string, driver: FixedDriver,
  progress?: StageProgress): Promise<StagedCli> {
  const item = fixedOfficialCli(driver);
  if (!item) throw new ManagedCliError("DRIVER", "unsupported driver");
  await plainDirectory(root);
  const stagePath = await mkdtemp(join(root, `${item.driver}-${item.version}-`));
  const archivePath = join(stagePath, "source.download");
  const contentPath = join(stagePath, "content");
  try {
    await mkdir(contentPath);
    await downloadPinned(item.url, archivePath, progress);
    progress?.("verifying", (await stat(archivePath)).size);
    if (await digestFile(archivePath) !== item.archiveSha256) {
      throw new ManagedCliError("ARCHIVE_SHA256", "official archive digest mismatch");
    }
    if (item.format === "tgz") {
      const tarPath = join(stagePath, "source.tar");
      await pipeline(createReadStream(archivePath), createGunzip(), createWriteStream(tarPath, { flags: "wx" }));
      await extractAllowlistedTar(tarPath, contentPath, item.files, progress);
      await rm(tarPath);
    } else {
      await rename(archivePath, join(contentPath, item.executable));
    }
    const imagePath = join(contentPath, ...item.executable.split("/"));
    const image = await lstat(imagePath);
    if (!image.isFile() || image.isSymbolicLink() || await digestFile(imagePath) !== item.executableSha256) {
      throw new ManagedCliError("IMAGE_SHA256", "official executable digest mismatch");
    }
    return Object.freeze({ driver: item.driver, version: item.version, stagePath,
      imagePath, archiveSha256: item.archiveSha256, imageSha256: item.executableSha256 });
  } catch (error) {
    await rm(stagePath, { recursive: true, force: true });
    throw error;
  }
}
