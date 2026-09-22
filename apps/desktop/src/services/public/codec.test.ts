import { describe, expect, it } from "vitest";
import { decodePublicJson, encodePublicJson, PublicCodecError } from "./codec";
import schemaText from "../../../contracts/s1/schema.json?raw";
import manifestText from "../../../contracts/s1/fixtures/manifest.json?raw";

type JsonObject = Record<string, unknown>;
type SchemaDefinition = JsonObject & { properties?: JsonObject; required?: string[] };
type SchemaDocument = {
  oneOf: Array<{ $ref: string }>;
  $defs: Record<string, SchemaDefinition & { const?: unknown }>;
};
type FixtureManifest = {
  schemaVersion: number;
  canonicalPositive: Array<{ file: string; target: string }>;
  negative: Array<{ file: string; encoding: "utf8" | "hex"; target: string; error: string }>;
  nativeProtocolSources: Array<{
    provider: "codex" | "claude" | "grok";
    sourcePath: string;
    sha256: string;
    sourceAnchor: string;
    representativeFrame: string;
    expectedError: string;
  }>;
};

const schema = JSON.parse(schemaText) as SchemaDocument;
const manifest = JSON.parse(manifestText) as FixtureManifest;
const fixtureSources = import.meta.glob("../../../contracts/s1/fixtures/**/*", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const nativeProtocolSourceFiles = import.meta.glob("../../../contracts/s1/fixtures/native-protocol-sources/*.mjs", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const fixtureRaw = (path: string): string => {
  const key = Object.keys(fixtureSources).find((candidate) => candidate.endsWith(`/fixtures/${path}`));
  if (!key) throw new Error(`fixture is not available: ${path}`);
  return fixtureSources[key];
};

const fixtureBytes = (entry: { file: string; encoding?: "utf8" | "hex" }): Uint8Array => {
  const source = fixtureRaw(entry.file).trimEnd();
  if (entry.encoding === "hex") {
    const pairs = source.match(/../g);
    if (!pairs) throw new Error(`hex fixture is empty: ${entry.file}`);
    return Uint8Array.from(pairs.map((pair) => Number.parseInt(pair, 16)));
  }
  return new TextEncoder().encode(source);
};

const nativeProtocolSourceRaw = (path: string): string => {
  const normalized = path.split("\\").join("/");
  const key = Object.keys(nativeProtocolSourceFiles).find((candidate) => candidate.split("\\").join("/").endsWith(normalized.replace(/^apps\/desktop\//u, "")));
  if (!key) throw new Error(`native protocol source is not available: ${path}`);
  return nativeProtocolSourceFiles[key];
};

const sha256Hex = async (source: string): Promise<string> => {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(source));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
};

const schemaVariantNames = schema.oneOf.map((variant) => {
  const prefix = "#/$defs/";
  if (!variant.$ref.startsWith(prefix)) throw new Error(`external schema ref: ${variant.$ref}`);
  return variant.$ref.slice(prefix.length);
});

const isObject = (value: unknown): value is JsonObject => value !== null && typeof value === "object" && !Array.isArray(value);

const schemaDefinitionMatches = (definition: SchemaDefinition, value: unknown): boolean => {
  if (!isObject(value) || !definition.properties || !definition.required) return false;
  if (definition.required.some((field) => !Object.prototype.hasOwnProperty.call(value, field))) return false;
  if (definition.additionalProperties === false && Object.keys(value).some((field) => !Object.prototype.hasOwnProperty.call(definition.properties, field))) return false;
  return Object.entries(definition.properties).every(([field, property]) => {
    if (!Object.prototype.hasOwnProperty.call(value, field)) return true;
    const actual = value[field];
    if (!isObject(property)) return true;
    if (Object.prototype.hasOwnProperty.call(property, "const") && actual !== property.const) return false;
    return !Array.isArray(property.enum) || property.enum.some((allowed) => allowed === actual);
  });
};

const schemaMatches = (value: unknown): string[] => schemaVariantNames.filter((name) => schemaDefinitionMatches(schema.$defs[name], value));

describe("S1 public codec fixtures", () => {
  it("pins positive integer schema versions and non-empty capability names in the schema", () => {
    expect(schema.$defs.SchemaVersion).toMatchObject({ type: "integer", minimum: 1, const: 1 });
    const capabilityProperties = schema.$defs.CapabilitySnapshot.properties;
    expect(capabilityProperties).toBeDefined();
    const capabilities = capabilityProperties?.capabilities;
    expect(isObject(capabilities)).toBe(true);
    if (!isObject(capabilities)) throw new Error("CapabilitySnapshot.capabilities schema is missing");
    expect(capabilities.propertyNames).toEqual({ type: "string", minLength: 1 });
    expect(schema.$defs.UtcTimestamp).toMatchObject({
      type: "string",
      format: "date-time",
      pattern: "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]([.][0-9]+)?(Z|[+]00:00)$",
    });
  });

  it("reads the manifest and schema, and every canonical positive matches oneOf exactly once", () => {
    expect(manifest.schemaVersion).toBe(schema.$defs.SchemaVersion.const);
    for (const name of schemaVariantNames) {
      expect(manifest.canonicalPositive.some((entry) => entry.target === name)).toBe(true);
    }
    for (const entry of manifest.canonicalPositive) {
      const raw = JSON.parse(fixtureRaw(entry.file)) as unknown;
      expect(schemaMatches(raw)).toEqual([entry.target]);
      const bytes = fixtureBytes(entry);
      expect(encodePublicJson(decodePublicJson(bytes))).toEqual(bytes);
    }
  });

  it("uses the manifest's declared target and error for every negative fixture", () => {
    for (const entry of manifest.negative) {
      expect(schemaVariantNames).toContain(entry.target);
      try {
        decodePublicJson(fixtureBytes(entry));
        throw new Error(`expected rejection: ${entry.file}`);
      } catch (error) {
        expect(error).toBeInstanceOf(PublicCodecError);
        expect((error as PublicCodecError).code).toBe(entry.error);
      }
    }
  });

  it("recognizes an escaped schemaVersion key as the decoded top-level field", () => {
    const canonical = fixtureBytes({ file: "positive/session.json" });
    const escaped = new TextEncoder().encode(
      fixtureRaw("positive/session.json").trimEnd().replace('"schemaVersion"', '"schema\\u0056ersion"'),
    );
    expect(encodePublicJson(decodePublicJson(escaped))).toEqual(canonical);
  });

  it("prioritizes duplicate keys over schema number semantics in both source orders", () => {
    for (const file of [
      "negative/duplicate-before-schema-version-fractional.json",
      "negative/schema-version-fractional-before-duplicate.json",
      "negative/generic-integer-overflow-before-escaped-duplicate.json",
    ]) {
      expect(() => decodePublicJson(fixtureBytes({ file }))).toThrowError(
        expect.objectContaining({ code: "DuplicateKey" }),
      );
    }
  });

  it("accepts the serde_json i64/u64 generic integer domain and rejects outside it", () => {
    const event = (token: string): Uint8Array => new TextEncoder().encode(
      `{"bindingId":null,"eventId":"00000000-0000-0000-0000-000000000011","executionId":null,"generation":"0","kind":"diagnostic","payload":{"value":${token}},"schemaVersion":1,"sequence":"1","sessionId":"00000000-0000-0000-0000-000000000001","streamEpoch":"1"}`,
    );
    for (const token of ["-9223372036854775808", "9223372036854775807", "9223372036854775808", "18446744073709551615"]) {
      expect(() => decodePublicJson(event(token))).not.toThrow();
    }
    for (const token of ["-9223372036854775809", "18446744073709551616"]) {
      expect(() => decodePublicJson(event(token))).toThrowError(expect.objectContaining({ code: "NonCanonicalNumber" }));
    }
  });

  it("accepts escaped surrogate pairs and rejects shared lone-surrogate fixtures", () => {
    const paired = fixtureRaw("positive/session.json").trimEnd().replace("S1 public session", "\\uD83D\\uDE00");
    expect(() => decodePublicJson(new TextEncoder().encode(paired))).not.toThrow();
    for (const file of [
      "negative/escaped-lone-high-surrogate.json",
      "negative/escaped-lone-low-surrogate.json",
    ]) {
      expect(() => decodePublicJson(fixtureBytes({ file }))).toThrowError(
        expect.objectContaining({ code: "InvalidJson" }),
      );
    }
  });

  it("enforces exact frame, UTF-8 title, and context item limits", () => {
    const canonicalSession = fixtureBytes({ file: "positive/session.json" });
    const atFrameLimit = new Uint8Array(4 * 1024 * 1024);
    atFrameLimit.fill(0x20);
    atFrameLimit.set(canonicalSession);
    expect(() => decodePublicJson(atFrameLimit)).not.toThrow();
    expect(() => decodePublicJson(new Uint8Array(4 * 1024 * 1024 + 1))).toThrowError(
      expect.objectContaining({ code: "FrameTooLarge" }),
    );

    const session = JSON.parse(fixtureRaw("positive/session.json")) as JsonObject;
    session.title = "é".repeat(128);
    expect(() => decodePublicJson(new TextEncoder().encode(JSON.stringify(session)))).not.toThrow();
    session.title = `${"é".repeat(128)}a`;
    expect(() => decodePublicJson(new TextEncoder().encode(JSON.stringify(session)))).toThrowError(
      expect.objectContaining({ code: "BoundsExceeded" }),
    );

    const context = JSON.parse(fixtureRaw("positive/context-package.json")) as JsonObject;
    const item = (context.items as unknown[])[0];
    context.items = Array.from({ length: 128 }, () => item);
    expect(() => decodePublicJson(new TextEncoder().encode(JSON.stringify(context)))).not.toThrow();
    context.items = Array.from({ length: 129 }, () => item);
    expect(() => decodePublicJson(new TextEncoder().encode(JSON.stringify(context)))).toThrowError(
      expect.objectContaining({ code: "BoundsExceeded" }),
    );
  });

  it("consumes digest-pinned native generator frames without granting public authority", async () => {
    expect(manifest.nativeProtocolSources).toHaveLength(5);
    expect(new Set(manifest.nativeProtocolSources.map((source) => source.provider))).toEqual(
      new Set(["codex", "claude", "grok"]),
    );
    for (const source of manifest.nativeProtocolSources) {
      expect(source.sourcePath.startsWith("apps/desktop/contracts/s1/fixtures/native-protocol-sources/")).toBe(true);
      expect(source.sourcePath).not.toContain("..");
      const sourceText = nativeProtocolSourceRaw(source.sourcePath);
      expect(await sha256Hex(sourceText)).toBe(source.sha256);
      expect(sourceText).toContain(source.sourceAnchor);
      expect(() => JSON.parse(source.representativeFrame)).not.toThrow();
      expect(() => decodePublicJson(new TextEncoder().encode(source.representativeFrame))).toThrowError(
        expect.objectContaining({ code: source.expectedError }),
      );
    }
  });
});
