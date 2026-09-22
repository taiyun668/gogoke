import { describe, expect, it } from "vitest";

import {
  applyRuntimeConfigChanges,
  buildRuntimeConfigViewModel,
  decodeRuntimeConfigUiSourceJson,
  RuntimeConfigUiError,
  validateRuntimeConfigSchema,
} from "./configForm";
import { GENERIC_RUNTIME_ICON, RUNTIME_CONFIG_UI_SCHEMA } from "./types";

const schema = {
  schema: RUNTIME_CONFIG_UI_SCHEMA,
  title: "Local runtime",
  sections: [
    {
      id: "launch",
      label: "Launch",
      fields: [
        {
          key: "binaryPath",
          label: "Binary path",
          control: "path" as const,
          required: true,
        },
        {
          key: "mode",
          label: "Mode",
          control: "select" as const,
          required: true,
          options: [
            { value: "managed", label: "Managed" },
            { value: "isolated", label: "Isolated" },
          ],
        },
        {
          key: "telemetry",
          label: "Share diagnostics",
          control: "toggle" as const,
          required: false,
        },
      ],
    },
  ],
};

const buildFromJson = (source: unknown) =>
  buildRuntimeConfigViewModel(decodeRuntimeConfigUiSourceJson(JSON.stringify(source)));

describe("R4 runtime config UI contract", () => {
  it("gogoke-s1-r4/R4-01 derives fields from schema and uses an accessible generic icon", () => {
    const viewModel = buildFromJson({
      driverId: "mock_novel_runtime",
      availability: { status: "available" },
      schema,
      config: { binaryPath: "C:\\tools\\runtime.exe", mode: "managed", telemetry: false },
    });

    expect(viewModel.icon).toEqual({
      key: GENERIC_RUNTIME_ICON,
      accessibleLabel: "mock_novel_runtime runtime",
      fallback: true,
    });
    expect(viewModel.editable).toBe(true);
    expect(viewModel.sections[0]?.fields.map((field) => field.key)).toEqual([
      "binaryPath",
      "mode",
      "telemetry",
    ]);
    expect(viewModel.sections[0]?.fields.every((field) => field.error === null)).toBe(true);
  });

  it("gogoke-s1-r4/R4-01 exposes unknown drivers as unavailable without dropping opaque config", () => {
    const config = { futureNested: { retain: ["all", 2] }, enabledByVendor: true };
    const viewModel = buildFromJson({
      driverId: "unknown_driver",
      displayName: "Unknown driver",
      availability: { status: "unavailable", reason: "DRIVER_NOT_REGISTERED" },
      config,
    });

    expect(viewModel.editable).toBe(false);
    expect(viewModel.sections).toEqual([]);
    expect(viewModel.opaqueConfig).toEqual(config);
    expect(applyRuntimeConfigChanges(viewModel, { futureNested: "overwrite" })).toEqual(config);
  });

  it("gogoke-s1-r4/R4-01 ignores untrusted schema and icon declarations for unavailable drivers", () => {
    const viewModel = buildFromJson({
      driverId: "unknown_driver",
      iconKey: "brand-shaped-icon",
      availability: { status: "unavailable", reason: "DRIVER_NOT_REGISTERED" },
      schema: { schema: "future.schema", futureMetadata: { retained: true } } as never,
      config: { preserved: true },
    });

    expect(viewModel.icon).toEqual({
      key: GENERIC_RUNTIME_ICON,
      accessibleLabel: "unknown_driver runtime",
      fallback: true,
    });
    expect(viewModel.sections).toEqual([]);
    expect(viewModel.opaqueConfig).toEqual({ preserved: true });
  });

  it("gogoke-s1-r4/R4-02 preserves fields outside the registered config schema", () => {
    const viewModel = buildFromJson({
      driverId: "mock_novel_runtime",
      iconKey: "adapter-provided-icon",
      availability: { status: "available" },
      schema,
      config: {
        binaryPath: "old.exe",
        mode: "managed",
        extensionOwned: { retained: true },
      },
    });

    expect(applyRuntimeConfigChanges(viewModel, { binaryPath: "new.exe" })).toEqual({
      binaryPath: "new.exe",
      mode: "managed",
      extensionOwned: { retained: true },
    });
  });

  it("gogoke-s1-r4/R4-03 keeps unavailable state explicit instead of presenting ready controls", () => {
    const viewModel = buildFromJson({
      driverId: "known_driver",
      availability: { status: "unavailable", reason: "ADAPTER_VERSION_NOT_REGISTERED" },
      schema,
      config: { binaryPath: "fixture.exe", mode: "managed" },
    });

    expect(viewModel.editable).toBe(false);
    expect(viewModel.availability).toEqual({
      status: "unavailable",
      reason: "ADAPTER_VERSION_NOT_REGISTERED",
    });
  });

  it("gogoke-s1-r4/R4-01 requires passive JSON source text", () => {
    expect(() => decodeRuntimeConfigUiSourceJson("{not-json")).toThrow(RuntimeConfigUiError);
    expect(() =>
      buildRuntimeConfigViewModel({
        driverId: "known_driver",
        availability: { status: "available" },
        schema,
        config: { binaryPath: "fixture.exe" },
      }),
    ).toThrowError(expect.objectContaining({ code: "INVALID_SOURCE" }));

    const branded = decodeRuntimeConfigUiSourceJson(
      JSON.stringify({
        driverId: "known_driver",
        availability: { status: "available" },
        schema,
        config: { binaryPath: "fixture.exe", mode: "managed" },
      }),
    );
    expect(buildRuntimeConfigViewModel(branded).editable).toBe(true);
    expect(() => buildRuntimeConfigViewModel({ ...branded })).toThrowError(
      expect.objectContaining({ code: "INVALID_SOURCE" }),
    );
    expect(() =>
      decodeRuntimeConfigUiSourceJson(
        JSON.stringify({
          driverId: "known_driver",
          availability: { status: "available" },
          schema,
          config: {},
          admissionRef: "forged-authority",
        }),
      ),
    ).toThrow(RuntimeConfigUiError);

    let proxyTraps = 0;
    const proxied = new Proxy(branded, {
      get: (target, property, receiver) => {
        proxyTraps += 1;
        return Reflect.get(target, property, receiver);
      },
      getPrototypeOf: (target) => {
        proxyTraps += 1;
        return Reflect.getPrototypeOf(target);
      },
      ownKeys: (target) => {
        proxyTraps += 1;
        return Reflect.ownKeys(target);
      },
      getOwnPropertyDescriptor: (target, property) => {
        proxyTraps += 1;
        return Reflect.getOwnPropertyDescriptor(target, property);
      },
    });
    expect(() => buildRuntimeConfigViewModel(proxied)).toThrowError(
      expect.objectContaining({ code: "INVALID_SOURCE" }),
    );
    expect(proxyTraps).toBe(0);
  });

  it("gogoke-s1-r4/R4-02 clones and freezes opaque history before edits", () => {
    const source = { extensionOwned: { values: ["keep"] } };
    const viewModel = buildFromJson({
      driverId: "mock_novel_runtime",
      availability: { status: "available" },
      schema,
      config: source,
    });

    source.extensionOwned.values.push("mutated-after-load");
    expect(viewModel.opaqueConfig).toEqual({ extensionOwned: { values: ["keep"] } });
    expect(Object.isFrozen(viewModel.opaqueConfig)).toBe(true);
    expect(Object.isFrozen(viewModel.opaqueConfig.extensionOwned)).toBe(true);
  });

  it("gogoke-s1-r4/R4-02 freezes the VM and keeps edit authority outside caller data", () => {
    const viewModel = buildFromJson({
      driverId: "mock_novel_runtime",
      availability: { status: "available" },
      schema,
      config: {
        binaryPath: "old.exe",
        mode: "managed",
        extensionOwned: { retained: true },
      },
    });
    const mutable = viewModel as unknown as {
      sections: Array<{ fields: Array<{ key: string }> }>;
    };

    expect(Object.isFrozen(viewModel)).toBe(true);
    expect(Object.isFrozen(viewModel.icon)).toBe(true);
    expect(Object.isFrozen(viewModel.availability)).toBe(true);
    expect(Object.isFrozen(viewModel.sections)).toBe(true);
    expect(Object.isFrozen(viewModel.sections[0])).toBe(true);
    expect(Object.isFrozen(viewModel.sections[0]?.fields)).toBe(true);
    expect(Object.isFrozen(viewModel.sections[0]?.fields[0])).toBe(true);
    expect(() => {
      mutable.sections[0]!.fields[0]!.key = "extensionOwned";
    }).toThrow(TypeError);
    expect(() => applyRuntimeConfigChanges(viewModel, { extensionOwned: "overwrite" })).toThrow(
      RuntimeConfigUiError,
    );
    expect(applyRuntimeConfigChanges(viewModel, { binaryPath: "new.exe" })).toEqual({
      binaryPath: "new.exe",
      mode: "managed",
      extensionOwned: { retained: true },
    });

    const forged = {
      ...viewModel,
      sections: [
        {
          ...viewModel.sections[0]!,
          fields: [{ ...viewModel.sections[0]!.fields[0]!, key: "extensionOwned" }],
        },
      ],
    };
    expect(() => applyRuntimeConfigChanges(forged, { extensionOwned: "overwrite" })).toThrowError(
      expect.objectContaining({ code: "INVALID_VIEW_MODEL" }),
    );
  });

  it("gogoke-s1-r4/R4-01 sanitizes the declarative schema and rejects unknown controls", () => {
    const mutableSchema = structuredClone(schema);
    const parsed = validateRuntimeConfigSchema(mutableSchema);
    mutableSchema.sections[0]!.fields[0]!.label = "Changed after validation";
    expect(parsed.sections[0]!.fields[0]!.label).toBe("Binary path");
    expect(Object.isFrozen(parsed.sections[0]!.fields)).toBe(true);

    expect(() =>
      validateRuntimeConfigSchema({
        ...schema,
        sections: [
          {
            id: "launch",
            label: "Launch",
            fields: [{ key: "script", label: "Script", control: "execute", required: false }],
          },
        ],
      }),
    ).toThrow(RuntimeConfigUiError);
  });

  it("gogoke-s1-r4/R4-01 snapshots source data without executing volatile getters", () => {
    const base: Record<string, unknown> = {
      driverId: "mock_novel_runtime",
      availability: { status: "available" },
      schema,
      config: { binaryPath: "fixture.exe", mode: "managed" },
    };
    for (const field of ["driverId", "availability", "schema", "config"]) {
      let reads = 0;
      const source: Record<string, unknown> = { ...base };
      const value = source[field];
      Object.defineProperty(source, field, {
        enumerable: true,
        get: () => {
          reads += 1;
          return value;
        },
      });
      expect(() => buildRuntimeConfigViewModel(source as never)).toThrow(RuntimeConfigUiError);
      expect(reads).toBe(0);
    }

    let configReads = 0;
    const accessorConfig: Record<string, unknown> = {
      binaryPath: "fixture.exe",
      mode: "managed",
    };
    Object.defineProperty(accessorConfig, "extensionOwned", {
      enumerable: true,
      get: () => {
        configReads += 1;
        return { forged: true };
      },
    });
    expect(() =>
      buildRuntimeConfigViewModel({
        driverId: "mock_novel_runtime",
        availability: { status: "available" },
        schema,
        config: accessorConfig,
      }),
    ).toThrow(RuntimeConfigUiError);
    expect(configReads).toBe(0);

    const viewModel = buildFromJson(base);
    let changeReads = 0;
    const changes: Record<string, unknown> = {};
    Object.defineProperty(changes, "binaryPath", {
      enumerable: true,
      get: () => {
        changeReads += 1;
        return "forged.exe";
      },
    });
    expect(() => applyRuntimeConfigChanges(viewModel, changes)).toThrow(RuntimeConfigUiError);
    expect(changeReads).toBe(0);

    const extendedArray = ["fixture.exe"] as unknown as Record<string, unknown>;
    Object.defineProperty(extendedArray, "4294967295", {
      value: () => "forged.exe",
      enumerable: true,
    });
    expect(() => applyRuntimeConfigChanges(viewModel, { binaryPath: extendedArray })).toThrow(
      RuntimeConfigUiError,
    );
  });

  it("gogoke-s1-r4/R4-01 rejects active source extras and keeps availability coherent", () => {
    const availability: Record<string, unknown> = { status: "available" };
    const source: Record<string, unknown> = {
      driverId: "mock_novel_runtime",
      availability,
      schema,
      config: { binaryPath: "fixture.exe", mode: "managed" },
    };
    const viewModel = buildFromJson(source);
    availability.status = "unavailable";
    availability.reason = "DRIVER_NOT_REGISTERED";
    expect(viewModel.availability).toEqual({ status: "available" });
    expect(viewModel.editable).toBe(true);

    const executableExtra = { ...source, lifecycleHook: () => undefined };
    expect(() => buildRuntimeConfigViewModel(executableExtra as never)).toThrow(
      RuntimeConfigUiError,
    );

    const symbolExtra = { ...source } as Record<PropertyKey, unknown>;
    symbolExtra[Symbol("authority")] = "forged";
    expect(() => buildRuntimeConfigViewModel(symbolExtra as never)).toThrow(RuntimeConfigUiError);
  });
});
