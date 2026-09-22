import { describe, expect, it } from "vitest";

import { authorityProjection, decodePublicObject, encodePublicObject } from "./codec.ts";
import {
  CONTEXT_SCOPES,
  CONTEXT_STATES,
  DECISION_STATES,
  DREAM_STATES,
  EXPOSURE_STATES,
  OBJECT_DEFINITIONS,
  OUTCOME_STATES,
  type NativeModelId,
  type RoleId,
  type RuntimeAccountRef,
  type RuntimeDriverId,
  type RuntimeInstanceId,
  type SeatId,
  type FieldRule,
  type JsonValue,
} from "./model.ts";
import { ContractCodecError } from "./strictJson.ts";

type ErrorPredicate = (error: unknown) => boolean;
type ErrorConstructor = new (...args: never[]) => Error;
type ThrowMatcher = ErrorPredicate | ErrorConstructor;

const assert = {
  equal(actual: unknown, expected: unknown): void {
    expect(actual).toBe(expected);
  },
  notEqual(actual: unknown, expected: unknown): void {
    expect(actual).not.toBe(expected);
  },
  deepEqual(actual: unknown, expected: unknown): void {
    expect(actual).toEqual(expected);
  },
  throws(fn: () => unknown, matcher?: ThrowMatcher, _message?: string): void {
    let caught = false;
    let error: unknown;
    try {
      fn();
    } catch (candidate) {
      caught = true;
      error = candidate;
    }
    expect(caught).toBe(true);
    if (!matcher) return;
    if (
      typeof matcher === "function" &&
      "prototype" in matcher &&
      matcher.prototype instanceof Error
    ) {
      expect(error).toBeInstanceOf(matcher as ErrorConstructor);
      return;
    }
    expect((matcher as ErrorPredicate)(error)).toBe(true);
  },
};

const fixtureSources = import.meta.glob("../../../../contracts/s1-r4/fixtures/**/*", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const legacyFixtureSources = import.meta.glob("../../../../contracts/s1/fixtures/**/*", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const contractSources = import.meta.glob("../../../../contracts/s1-r4/*.json", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;
const normativeSources = import.meta.glob(
  "../../../../../../docs/design/gogoke-s1-r4-plan-v1/OBJECT_MODEL.json",
  { eager: true, query: "?raw", import: "default" },
) as Record<string, string>;

const sourceText = (sources: Record<string, string>, suffix: string): string => {
  const normalizedSuffix = suffix.split("\\").join("/");
  const key = Object.keys(sources).find((candidate) =>
    candidate.split("\\").join("/").endsWith(normalizedSuffix),
  );
  if (!key) throw new Error(`fixture is not available: ${suffix}`);
  return sources[key]!;
};

const fixture = (name: string): Uint8Array =>
  new TextEncoder().encode(sourceText(fixtureSources, `/fixtures/${name}`));

const legacyFixture = (name: string): Uint8Array =>
  new TextEncoder().encode(sourceText(legacyFixtureSources, `/fixtures/${name}`));

const contract = JSON.parse(sourceText(contractSources, "/object-model.v1.json")) as {
  objects: Record<string, Array<string>>;
  states: Record<string, Array<string>>;
};
const normative = JSON.parse(
  sourceText(normativeSources, "/OBJECT_MODEL.json"),
) as {
  objects: Record<string, Array<string>>;
  states: Record<string, Array<string>>;
};
const oracle = JSON.parse(sourceText(contractSources, "/field-rules.v1.json")) as {
  objects: Record<string, Record<string, FieldRule>>;
};

const expectCodecError = (name: string, code: ContractCodecError["code"]): void => {
  assert.throws(
    () => decodePublicObject(fixture(name)),
    (error: unknown) => error instanceof ContractCodecError && error.code === code,
  );
};

function assertIdentityBrandsAreNotInterchangeable(
  driverId: RuntimeDriverId,
  instanceId: RuntimeInstanceId,
  modelId: NativeModelId,
  roleId: RoleId,
  seatId: SeatId,
  accountRef: RuntimeAccountRef,
): void {
  // @ts-expect-error driver identity is not an instance identity
  const instanceFromDriver: RuntimeInstanceId = driverId;
  // @ts-expect-error model identity is not a driver identity
  const driverFromModel: RuntimeDriverId = modelId;
  // @ts-expect-error seat identity is not a role identity
  const roleFromSeat: RoleId = seatId;
  // @ts-expect-error account reference is not a seat identity
  const seatFromAccount: SeatId = accountRef;
  void instanceId;
  void roleId;
  void instanceFromDriver;
  void driverFromModel;
  void roleFromSeat;
  void seatFromAccount;
}

void assertIdentityBrandsAreNotInterchangeable;

const sampleFor = (rule: FieldRule): JsonValue => {
  if (typeof rule !== "string") return rule[1][0]!;
  switch (rule) {
    case "boolean":
      return false;
    case "driverId":
      return "mock_novel_seed";
    case "instanceId":
      return "instance_seed";
    case "json":
      return { fixture: true };
    case "jsonArray":
      return [{ fixture: true }];
    case "jsonObject":
      return { fixture: true };
    case "string":
      return "fixture";
    case "stringArray":
      return ["fixture"];
    case "u64":
      return "1";
  }
};

describe("S1-R4 public object byte codec", () => {
  it("preserves unknown minor fields outside the authority projection", () => {
    const decoded = decodePublicObject(fixture("runtime-instance.valid.json"));
    assert.equal(decoded.value.objectType, "RuntimeInstance");
    assert.equal(decoded.value.schema, "gogoke.s1-r4.objects.v1.1");
    assert.equal(Object.prototype.hasOwnProperty.call(authorityProjection(decoded), "futureDisplayHint"), false);
    assert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.object)), {
      accountRef: "account_ref_7",
      futureDisplayHint: "not-authority",
    });
    assert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.envelope)), {
      transportHint: "preserve-me",
    });

    const roundTrip = decodePublicObject(encodePublicObject(decoded));
    assert.deepEqual(roundTrip, decoded);
  });

  it("rejects duplicate keys before object validation", () => {
    expectCodecError("runtime-instance.duplicate-key.invalid.json", "DUPLICATE_KEY");
  });

  it("rejects malformed UTF-8 and truncated frames at the byte boundary", () => {
    assert.throws(
      () => decodePublicObject(Uint8Array.from([0xff, 0xfe])),
      (error: unknown) => error instanceof ContractCodecError && error.code === "INVALID_UTF8",
    );
    assert.throws(
      () => decodePublicObject(new TextEncoder().encode('{"schema":"gogoke.s1-r4.objects.v1"')),
      (error: unknown) => error instanceof ContractCodecError && error.code === "INVALID_JSON",
    );
  });

  it("rejects unknown majors, numeric u64 values, and u64 overflow", () => {
    expectCodecError("runtime-instance.unknown-major.invalid.json", "UNKNOWN_MAJOR");
    expectCodecError("runtime-instance.numeric-u64.invalid.json", "U64_NOT_STRING");
    expectCodecError("runtime-instance.overflow.invalid.json", "U64_OVERFLOW");
  });

  it("rejects lossy integer JSON numbers even in unknown fields", () => {
    const bytes = new TextEncoder().encode(
      '{"schema":"gogoke.s1-r4.objects.v1","objectType":"RuntimeDriver","unsafe":9007199254740992,"object":{"driverId":"driver","adapterVersion":"1","artifactDigest":"sha256:fixture","configSchemaRef":"schema","requiredHostServices":["host"],"admissionRef":"admission"}}',
    );
    assert.throws(
      () => decodePublicObject(bytes),
      (error: unknown) =>
        error instanceof ContractCodecError && error.code === "UNSAFE_JSON_NUMBER",
    );
  });

  it("accepts every v1 object using open provider-neutral IDs", () => {
    for (const [objectType, definition] of Object.entries(OBJECT_DEFINITIONS)) {
      const object = Object.fromEntries(
        Object.entries(definition).map(([field, rule]) => [field, sampleFor(rule)]),
      );
      const bytes = new TextEncoder().encode(
        JSON.stringify({ schema: "gogoke.s1-r4.objects.v1", objectType, object }),
      );
      assert.equal(decodePublicObject(bytes).value.objectType, objectType);
    }

    const modelBytes = new TextEncoder().encode(
      JSON.stringify({
        schema: "gogoke.s1-r4.objects.v1",
        objectType: "ModelRef",
        object: {
          runtimeInstanceId: "instance_seed",
          nativeModelId: "vendor/model.v2",
          resolvedVersion: "2026.09",
          capabilityRevision: "1",
        },
      }),
    );
    assert.equal(decodePublicObject(modelBytes).value.objectType, "ModelRef");
  });

  it("does not reinterpret the legacy multi-protocol event as a public object", () => {
    const legacy = legacyFixture("positive/public-event-no-native-thread-turn.json");
    assert.throws(
      () => decodePublicObject(legacy),
      (error: unknown) => error instanceof ContractCodecError && error.code === "INVALID_ENVELOPE",
    );
  });

  it("keeps the checked-in vocabulary aligned with executable definitions", () => {
    assert.equal(Object.keys(contract.objects).length, 16);
    assert.equal(Object.prototype.hasOwnProperty.call(contract.objects, "RuntimeAccount"), false);
    assert.deepEqual(contract.objects, normative.objects);
    assert.deepEqual(contract.states, normative.states);
    assert.deepEqual(Object.keys(contract.objects), Object.keys(OBJECT_DEFINITIONS));
    for (const [objectType, fields] of Object.entries(contract.objects)) {
      assert.deepEqual(
        fields,
        Object.keys(OBJECT_DEFINITIONS[objectType as keyof typeof OBJECT_DEFINITIONS]),
      );
    }
    assert.deepEqual(contract.states, {
      exposure: EXPOSURE_STATES,
      context: CONTEXT_STATES,
      decision: DECISION_STATES,
      outcome: OUTCOME_STATES,
      dream: DREAM_STATES,
    });
    assert.deepEqual(CONTEXT_SCOPES, ["GLOBAL", "PROJECT", "SESSION"]);
  });

  it("keeps every executable field rule aligned with the independent contract oracle", () => {
    assert.deepEqual(OBJECT_DEFINITIONS, oracle.objects);

    const invalidFor = (rule: FieldRule): JsonValue | undefined => {
      if (typeof rule !== "string") return "__invalid_enum__";
      switch (rule) {
        case "boolean":
          return "not-boolean";
        case "driverId":
        case "instanceId":
          return "9invalid";
        case "json":
          return undefined;
        case "jsonArray":
          return {};
        case "jsonObject":
          return [];
        case "string":
          return "";
        case "stringArray":
          return [""];
        case "u64":
          return "not-u64";
      }
    };

    for (const [objectType, definition] of Object.entries(oracle.objects)) {
      const baseline = Object.fromEntries(
        Object.entries(definition).map(([field, rule]) => [field, sampleFor(rule)]),
      );
      for (const [field, rule] of Object.entries(definition)) {
        const invalid = invalidFor(rule);
        if (invalid === undefined) continue;
        const bytes = new TextEncoder().encode(
          JSON.stringify({
            schema: "gogoke.s1-r4.objects.v1",
            objectType,
            object: { ...baseline, [field]: invalid },
          }),
        );
        assert.throws(
          () => decodePublicObject(bytes),
          ContractCodecError,
          `${objectType}.${field} must enforce ${JSON.stringify(rule)}`,
        );
      }
    }
  });

  it("does not treat unknown authority-shaped fields as authority", () => {
    const bytes = new TextEncoder().encode(
      JSON.stringify({
        schema: "gogoke.s1-r4.objects.v1.1",
        objectType: "RuntimeDriver",
        authority: { grantRef: "owner-grant" },
        object: {
          driverId: "mock_novel_seed",
          adapterVersion: "1",
          artifactDigest: "sha256:fixture",
          configSchemaRef: "schema",
          requiredHostServices: ["host"],
          admissionRef: "admission",
          grantRef: "owner-grant",
        },
      }),
    );
    const decoded = decodePublicObject(bytes);
    assert.equal(Object.prototype.hasOwnProperty.call(decoded.value.object, "grantRef"), false);
    assert.equal(Object.prototype.hasOwnProperty.call(decoded.value.object, "authority"), false);
    assert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.object)), {
      grantRef: "owner-grant",
    });
    const unknownEnvelope = Object.fromEntries(Object.entries(decoded.unknownFields.envelope));
    assert.deepEqual(Object.keys(unknownEnvelope), ["authority"]);
    assert.equal((unknownEnvelope.authority as { grantRef: string }).grantRef, "owner-grant");
    assert.equal(Object.prototype.hasOwnProperty.call(authorityProjection(decoded), "grantRef"), false);

    const encoded = encodePublicObject({
      ...decoded,
      unknownFields: {
        ...decoded.unknownFields,
        object: { ...decoded.unknownFields.object, admissionRef: "forged" },
      },
    });
    const roundTrip = decodePublicObject(encoded);
    assert.equal(
      (roundTrip.value.object as { readonly admissionRef: string }).admissionRef,
      "admission",
    );
    assert.equal(Object.prototype.hasOwnProperty.call(authorityProjection(roundTrip), "grantRef"), false);
  });

  it("returns a defensive deeply frozen authority projection", () => {
    const decoded = decodePublicObject(
      new TextEncoder().encode(
        JSON.stringify({
          schema: "gogoke.s1-r4.objects.v1",
          objectType: "RuntimeDriver",
          object: {
            driverId: "mock_novel_seed",
            adapterVersion: "1",
            artifactDigest: "sha256:fixture",
            configSchemaRef: "schema",
            requiredHostServices: ["host"],
            admissionRef: "admission",
          },
        }),
      ),
    );
    const projection = authorityProjection(decoded) as unknown as {
      admissionRef: string;
      requiredHostServices: string[];
    };

    assert.notEqual(projection, decoded.value.object);
    assert.equal(Object.isFrozen(projection), true);
    assert.equal(Object.isFrozen(projection.requiredHostServices), true);
    assert.throws(() => {
      projection.admissionRef = "forged";
    }, TypeError);
    assert.throws(() => {
      projection.requiredHostServices.push("evil");
    }, TypeError);
    assert.equal(projection.admissionRef, "admission");
    assert.deepEqual(projection.requiredHostServices, ["host"]);
  });

  it("freezes the validated decoded record before callers can forge authority", () => {
    const decoded = decodePublicObject(
      new TextEncoder().encode(
        JSON.stringify({
          schema: "gogoke.s1-r4.objects.v1",
          objectType: "RuntimeDriver",
          object: {
            driverId: "mock_novel_seed",
            adapterVersion: "1",
            artifactDigest: "sha256:fixture",
            configSchemaRef: "schema",
            requiredHostServices: ["host"],
            admissionRef: "admission",
          },
        }),
      ),
    );
    const mutable = decoded as unknown as {
      value: {
        object: {
          admissionRef: string;
          requiredHostServices: string[];
        };
      };
    };

    assert.equal(Object.isFrozen(decoded), true);
    assert.equal(Object.isFrozen(decoded.value), true);
    assert.equal(Object.isFrozen(decoded.value.object), true);
    assert.equal(Object.isFrozen(mutable.value.object.requiredHostServices), true);
    assert.throws(() => {
      mutable.value.object.admissionRef = "forged-before-projection";
    }, TypeError);
    assert.throws(() => {
      mutable.value.object.requiredHostServices.push("evil-before-projection");
    }, TypeError);

    const projection = authorityProjection(decoded) as unknown as {
      admissionRef: string;
      requiredHostServices: string[];
    };
    assert.equal(projection.admissionRef, "admission");
    assert.deepEqual(projection.requiredHostServices, ["host"]);
  });

  it("rejects structurally reconstructed records at the authority boundary", () => {
    const decoded = decodePublicObject(fixture("runtime-instance.valid.json"));
    const reconstructed = {
      ...decoded,
      value: {
        ...decoded.value,
        object: { ...decoded.value.object, hostRef: "forged-without-decode" },
      },
    } as typeof decoded;
    assert.throws(
      () => authorityProjection(reconstructed),
      (error: unknown) =>
        error instanceof ContractCodecError && error.code === "INVALID_ENVELOPE",
    );
  });

  it("returns ordinary frozen DTOs and freezes runtime validation tables", () => {
    const decoded = decodePublicObject(fixture("runtime-instance.valid.json"));
    assert.equal(Object.getPrototypeOf(decoded), Object.prototype);
    assert.equal(Object.getPrototypeOf(decoded.value.object), Object.prototype);
    assert.equal(Object.prototype.hasOwnProperty.call(decoded.value.object, "instanceId"), true);
    assert.equal(decoded.value.object.toString(), "[object Object]");
    assert.equal(Object.getPrototypeOf(structuredClone(decoded.value.object)), Object.prototype);
    assert.equal(Object.isFrozen(OBJECT_DEFINITIONS), true);
    assert.equal(Object.isFrozen(OBJECT_DEFINITIONS.ContextObject), true);
    assert.equal(Object.isFrozen(CONTEXT_STATES), true);
    assert.throws(() => {
      (OBJECT_DEFINITIONS.ContextObject as Record<string, FieldRule>).version = "string";
    }, TypeError);
    assert.throws(() => {
      (CONTEXT_STATES as unknown as string[]).push("FORGED_ACTIVE");
    }, TypeError);
  });
});
