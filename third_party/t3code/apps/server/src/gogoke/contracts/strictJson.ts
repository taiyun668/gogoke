import type { JsonObject, JsonValue } from "./model.ts";

export type ContractCodecErrorCode =
  | "DUPLICATE_KEY"
  | "INVALID_ENVELOPE"
  | "INVALID_FIELD"
  | "INVALID_JSON"
  | "INVALID_UTF8"
  | "U64_NOT_STRING"
  | "U64_OVERFLOW"
  | "UNKNOWN_MAJOR"
  | "UNKNOWN_OBJECT_TYPE"
  | "UNKNOWN_SCHEMA"
  | "UNSAFE_JSON_NUMBER";

export class ContractCodecError extends Error {
  override readonly name = "ContractCodecError";
  readonly code: ContractCodecErrorCode;
  readonly offset: number | undefined;

  constructor(code: ContractCodecErrorCode, message: string, offset?: number) {
    super(message);
    this.code = code;
    this.offset = offset;
  }
}

const isWhitespace = (character: string | undefined): boolean =>
  character === " " || character === "\n" || character === "\r" || character === "\t";

class StrictJsonParser {
  private offset = 0;
  private readonly text: string;

  constructor(text: string) {
    this.text = text;
  }

  parse(): JsonValue {
    this.skipWhitespace();
    const value = this.parseValue();
    this.skipWhitespace();
    if (this.offset !== this.text.length) {
      this.fail("Trailing content after JSON value");
    }
    return value;
  }

  private parseValue(): JsonValue {
    const current = this.text[this.offset];
    if (current === "{") return this.parseObject();
    if (current === "[") return this.parseArray();
    if (current === '"') return this.parseString();
    if (current === "t") return this.parseLiteral("true", true);
    if (current === "f") return this.parseLiteral("false", false);
    if (current === "n") return this.parseLiteral("null", null);
    if (current === "-" || (current !== undefined && current >= "0" && current <= "9")) {
      return this.parseNumber();
    }
    this.fail("Expected a JSON value");
  }

  private parseObject(): JsonObject {
    this.offset += 1;
    this.skipWhitespace();
    const result: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
    const keys = new Set<string>();
    if (this.text[this.offset] === "}") {
      this.offset += 1;
      return result;
    }
    while (true) {
      if (this.text[this.offset] !== '"') this.fail("Expected an object key");
      const keyOffset = this.offset;
      const key = this.parseString();
      if (keys.has(key)) {
        throw new ContractCodecError(
          "DUPLICATE_KEY",
          `Duplicate JSON key ${JSON.stringify(key)}`,
          keyOffset,
        );
      }
      keys.add(key);
      this.skipWhitespace();
      if (this.text[this.offset] !== ":") this.fail("Expected ':' after object key");
      this.offset += 1;
      this.skipWhitespace();
      result[key] = this.parseValue();
      this.skipWhitespace();
      const delimiter = this.text[this.offset];
      if (delimiter === "}") {
        this.offset += 1;
        return result;
      }
      if (delimiter !== ",") this.fail("Expected ',' or '}' in object");
      this.offset += 1;
      this.skipWhitespace();
    }
  }

  private parseArray(): ReadonlyArray<JsonValue> {
    this.offset += 1;
    this.skipWhitespace();
    const result: Array<JsonValue> = [];
    if (this.text[this.offset] === "]") {
      this.offset += 1;
      return result;
    }
    while (true) {
      result.push(this.parseValue());
      this.skipWhitespace();
      const delimiter = this.text[this.offset];
      if (delimiter === "]") {
        this.offset += 1;
        return result;
      }
      if (delimiter !== ",") this.fail("Expected ',' or ']' in array");
      this.offset += 1;
      this.skipWhitespace();
    }
  }

  private parseString(): string {
    const start = this.offset;
    this.offset += 1;
    while (this.offset < this.text.length) {
      const character = this.text[this.offset];
      if (character === '"') {
        this.offset += 1;
        const token = this.text.slice(start, this.offset);
        try {
          return JSON.parse(token) as string;
        } catch {
          throw new ContractCodecError("INVALID_JSON", "Invalid JSON string escape", start);
        }
      }
      if (character === "\\") {
        this.offset += 2;
      } else {
        this.offset += 1;
      }
    }
    this.fail("Unterminated JSON string", start);
  }

  private parseLiteral<T extends JsonValue>(token: string, value: T): T {
    if (this.text.slice(this.offset, this.offset + token.length) !== token) {
      this.fail(`Invalid literal; expected ${token}`);
    }
    this.offset += token.length;
    return value;
  }

  private parseNumber(): number {
    const start = this.offset;
    const token = this.text.slice(start).match(/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/)?.[0];
    if (token === undefined) this.fail("Invalid JSON number");
    this.offset += token.length;
    const value = Number(token);
    if (!Number.isFinite(value)) {
      throw new ContractCodecError("UNSAFE_JSON_NUMBER", "JSON number is not finite", start);
    }
    if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
      throw new ContractCodecError(
        "UNSAFE_JSON_NUMBER",
        "Integer JSON numbers outside the safe range must be decimal strings",
        start,
      );
    }
    return value;
  }

  private skipWhitespace(): void {
    while (isWhitespace(this.text[this.offset])) this.offset += 1;
  }

  private fail(message: string, offset = this.offset): never {
    throw new ContractCodecError("INVALID_JSON", message, offset);
  }
}

export function parseStrictJsonBytes(bytes: Uint8Array): JsonValue {
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new ContractCodecError("INVALID_UTF8", "Contract bytes are not valid UTF-8");
  }
  return new StrictJsonParser(text).parse();
}

export function canonicalJson(value: JsonValue): string {
  if (value === null || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
      throw new ContractCodecError(
        "UNSAFE_JSON_NUMBER",
        "Canonical JSON cannot contain a non-finite or unsafe integer",
      );
    }
    return JSON.stringify(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const object = value as JsonObject;
  return `{${Object.keys(object)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key]!)}`)
    .join(",")}}`;
}
