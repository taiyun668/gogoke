import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import { describe, it } from "vite-plus/test";

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

const fixtures = new URL(
  "../../../../../../../apps/desktop/contracts/s1-r4/fixtures/",
  import.meta.url,
);
const contractUrl = new URL(
  "../../../../../../../apps/desktop/contracts/s1-r4/object-model.v1.json",
  import.meta.url,
);
const fieldRulesUrl = new URL(
  "../../../../../../../apps/desktop/contracts/s1-r4/field-rules.v1.json",
  import.meta.url,
);
const normativeContractUrl = new URL(
  "../../../../../../../docs/design/gogoke-s1-r4-plan-v1/OBJECT_MODEL.json",
  import.meta.url,
);

const fixture = (name: string): Uint8Array => NodeFS.readFileSync(new URL(name, fixtures));

const expectCodecError = (name: string, code: ContractCodecError["code"]): void => {
  NodeAssert.throws(
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
    NodeAssert.equal(decoded.value.objectType, "RuntimeInstance");
    NodeAssert.equal(decoded.value.schema, "gogoke.s1-r4.objects.v1.1");
    NodeAssert.equal(Object.hasOwn(authorityProjection(decoded), "futureDisplayHint"), false);
    NodeAssert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.object)), {
      accountRef: "account_ref_7",
      futureDisplayHint: "not-authority",
    });
    NodeAssert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.envelope)), {
      transportHint: "preserve-me",
    });

    const roundTrip = decodePublicObject(encodePublicObject(decoded));
    NodeAssert.deepEqual(roundTrip, decoded);
  });

  it("rejects duplicate keys before object validation", () => {
    expectCodecError("runtime-instance.duplicate-key.invalid.json", "DUPLICATE_KEY");
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
    NodeAssert.throws(
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
      NodeAssert.equal(decodePublicObject(bytes).value.objectType, objectType);
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
    NodeAssert.equal(decodePublicObject(modelBytes).value.objectType, "ModelRef");
  });

  it("keeps the checked-in vocabulary aligned with executable definitions", () => {
    const contract = JSON.parse(NodeFS.readFileSync(contractUrl, "utf8")) as {
      objects: Record<string, Array<string>>;
      states: Record<string, Array<string>>;
    };
    const normative = JSON.parse(NodeFS.readFileSync(normativeContractUrl, "utf8")) as {
      objects: Record<string, Array<string>>;
      states: Record<string, Array<string>>;
    };
    NodeAssert.equal(Object.keys(contract.objects).length, 16);
    NodeAssert.equal(Object.hasOwn(contract.objects, "RuntimeAccount"), false);
    NodeAssert.deepEqual(contract.objects, normative.objects);
    NodeAssert.deepEqual(contract.states, normative.states);
    NodeAssert.deepEqual(Object.keys(contract.objects), Object.keys(OBJECT_DEFINITIONS));
    for (const [objectType, fields] of Object.entries(contract.objects)) {
      NodeAssert.deepEqual(
        fields,
        Object.keys(OBJECT_DEFINITIONS[objectType as keyof typeof OBJECT_DEFINITIONS]),
      );
    }
    NodeAssert.deepEqual(contract.states, {
      exposure: EXPOSURE_STATES,
      context: CONTEXT_STATES,
      decision: DECISION_STATES,
      outcome: OUTCOME_STATES,
      dream: DREAM_STATES,
    });
    NodeAssert.deepEqual(CONTEXT_SCOPES, ["GLOBAL", "PROJECT", "SESSION"]);
  });

  it("keeps every executable field rule aligned with the independent contract oracle", () => {
    const oracle = JSON.parse(NodeFS.readFileSync(fieldRulesUrl, "utf8")) as {
      objects: Record<string, Record<string, FieldRule>>;
    };
    NodeAssert.deepEqual(OBJECT_DEFINITIONS, oracle.objects);

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
        NodeAssert.throws(
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
    NodeAssert.equal(Object.hasOwn(decoded.value.object, "grantRef"), false);
    NodeAssert.equal(Object.hasOwn(decoded.value.object, "authority"), false);
    NodeAssert.deepEqual(Object.fromEntries(Object.entries(decoded.unknownFields.object)), {
      grantRef: "owner-grant",
    });
    const unknownEnvelope = Object.fromEntries(Object.entries(decoded.unknownFields.envelope));
    NodeAssert.deepEqual(Object.keys(unknownEnvelope), ["authority"]);
    NodeAssert.equal((unknownEnvelope.authority as { grantRef: string }).grantRef, "owner-grant");
    NodeAssert.equal(Object.hasOwn(authorityProjection(decoded), "grantRef"), false);

    const encoded = encodePublicObject({
      ...decoded,
      unknownFields: {
        ...decoded.unknownFields,
        object: { ...decoded.unknownFields.object, admissionRef: "forged" },
      },
    });
    const roundTrip = decodePublicObject(encoded);
    NodeAssert.equal(
      (roundTrip.value.object as { readonly admissionRef: string }).admissionRef,
      "admission",
    );
    NodeAssert.equal(Object.hasOwn(authorityProjection(roundTrip), "grantRef"), false);
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

    NodeAssert.notEqual(projection, decoded.value.object);
    NodeAssert.equal(Object.isFrozen(projection), true);
    NodeAssert.equal(Object.isFrozen(projection.requiredHostServices), true);
    NodeAssert.throws(() => {
      projection.admissionRef = "forged";
    }, TypeError);
    NodeAssert.throws(() => {
      projection.requiredHostServices.push("evil");
    }, TypeError);
    NodeAssert.equal(projection.admissionRef, "admission");
    NodeAssert.deepEqual(projection.requiredHostServices, ["host"]);
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

    NodeAssert.equal(Object.isFrozen(decoded), true);
    NodeAssert.equal(Object.isFrozen(decoded.value), true);
    NodeAssert.equal(Object.isFrozen(decoded.value.object), true);
    NodeAssert.equal(Object.isFrozen(mutable.value.object.requiredHostServices), true);
    NodeAssert.throws(() => {
      mutable.value.object.admissionRef = "forged-before-projection";
    }, TypeError);
    NodeAssert.throws(() => {
      mutable.value.object.requiredHostServices.push("evil-before-projection");
    }, TypeError);

    const projection = authorityProjection(decoded) as unknown as {
      admissionRef: string;
      requiredHostServices: string[];
    };
    NodeAssert.equal(projection.admissionRef, "admission");
    NodeAssert.deepEqual(projection.requiredHostServices, ["host"]);
  });

  it("rejects structurally reconstructed records at the authority boundary", () => {
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
    const reconstructed = {
      ...decoded,
      value: {
        ...decoded.value,
        object: { ...decoded.value.object, admissionRef: "forged-without-decode" },
      },
    } as typeof decoded;

    NodeAssert.throws(
      () => authorityProjection(reconstructed),
      (error: unknown) => error instanceof ContractCodecError && error.code === "INVALID_ENVELOPE",
    );
  });

  it("returns ordinary frozen DTOs while keeping parser internals private", () => {
    const decoded = decodePublicObject(fixture("runtime-instance.valid.json"));
    NodeAssert.equal(Object.getPrototypeOf(decoded), Object.prototype);
    NodeAssert.equal(Object.getPrototypeOf(decoded.value.object), Object.prototype);
    NodeAssert.equal(Object.prototype.hasOwnProperty.call(decoded.value.object, "instanceId"), true);
    NodeAssert.equal(decoded.value.object.toString(), "[object Object]");
    NodeAssert.equal(Object.getPrototypeOf(structuredClone(decoded.value.object)), Object.prototype);
  });

  it("freezes exported validation tables and enum vocabularies at runtime", () => {
    NodeAssert.equal(Object.isFrozen(OBJECT_DEFINITIONS), true);
    NodeAssert.equal(Object.isFrozen(OBJECT_DEFINITIONS.ContextObject), true);
    NodeAssert.equal(Object.isFrozen(CONTEXT_STATES), true);
    NodeAssert.throws(() => {
      (OBJECT_DEFINITIONS.ContextObject as Record<string, FieldRule>).version = "string";
    }, TypeError);
    NodeAssert.throws(() => {
      (CONTEXT_STATES as unknown as string[]).push("FORGED_ACTIVE");
    }, TypeError);
  });
});
