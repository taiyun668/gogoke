import {
  GENERIC_RUNTIME_ICON,
  RUNTIME_CONFIG_UI_SCHEMA,
  type RuntimeConfigData,
  type RuntimeConfigFieldSchema,
  type RuntimeConfigFieldViewModel,
  type RuntimeConfigSchemaV1,
  type RuntimeConfigUiSource,
  type RuntimeConfigValue,
  type RuntimeConfigViewModel,
} from "./types";

const OPEN_ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const FIELD_KEY_PATTERN = /^[A-Za-z][A-Za-z0-9_.-]{0,127}$/;

export class RuntimeConfigUiError extends Error {
  constructor(
    readonly code:
      | "INVALID_CONFIG"
      | "INVALID_SCHEMA"
      | "INVALID_SOURCE"
      | "INVALID_VIEW_MODEL"
      | "UNKNOWN_FIELD",
    message: string,
  ) {
    super(message);
    this.name = "RuntimeConfigUiError";
  }
}

interface RuntimeConfigEditPolicy {
  readonly editable: boolean;
  readonly knownFields: ReadonlySet<string>;
  readonly opaqueConfig: RuntimeConfigData;
}

const editPolicies = new WeakMap<RuntimeConfigViewModel, RuntimeConfigEditPolicy>();
const authenticSources = new WeakSet<object>();

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const plainDataDescriptors = (
  value: object,
  path: string,
): Readonly<Record<PropertyKey, PropertyDescriptor>> => {
  const prototype = Object.getPrototypeOf(value);
  if (
    (Array.isArray(value) && prototype !== Array.prototype) ||
    (!Array.isArray(value) && prototype !== Object.prototype && prototype !== null)
  ) {
    throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must be a plain data object`);
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  for (const key of Reflect.ownKeys(descriptors)) {
    if (typeof key === "symbol") {
      throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must not contain symbol keys`);
    }
    const descriptor = descriptors[key]!;
    if (!("value" in descriptor) || !descriptor.enumerable) {
      if (Array.isArray(value) && key === "length" && "value" in descriptor) continue;
      throw new RuntimeConfigUiError(
        "INVALID_CONFIG",
        `${path}.${key} must be an enumerable data property`,
      );
    }
  }
  return descriptors;
};

const cloneConfigValue = (
  value: unknown,
  path: string,
  ancestors: WeakSet<object>,
): RuntimeConfigValue | ReadonlyArray<unknown> | RuntimeConfigData => {
  if (value === null || typeof value === "boolean" || typeof value === "string") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must contain finite numbers`);
    }
    return value;
  }
  if (typeof value !== "object") {
    throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must contain JSON data`);
  }
  if (ancestors.has(value)) {
    throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must not contain a cycle`);
  }
  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const descriptors = plainDataDescriptors(value, path);
      const length = descriptors.length?.value as number;
      const cloned: unknown[] = [];
      const allowedKeys = new Set<string>(["length"]);
      for (let index = 0; index < length; index += 1) {
        allowedKeys.add(String(index));
        const descriptor = descriptors[String(index)];
        if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
          throw new RuntimeConfigUiError(
            "INVALID_CONFIG",
            `${path}[${index}] must be an enumerable data property`,
          );
        }
        cloned.push(cloneConfigValue(descriptor.value, `${path}[${index}]`, ancestors));
      }
      const unexpected = Reflect.ownKeys(descriptors).find(
        (key) => typeof key === "string" && !allowedKeys.has(key),
      );
      if (unexpected !== undefined) {
        throw new RuntimeConfigUiError(
          "INVALID_CONFIG",
          `${path} contains unexpected property ${String(unexpected)}`,
        );
      }
      return Object.freeze(cloned);
    }
    const descriptors = plainDataDescriptors(value, path);
    const cloned: Record<string, unknown> = {};
    for (const key of Object.keys(descriptors)) {
      const descriptor = descriptors[key]!;
      Object.defineProperty(cloned, key, {
        value: cloneConfigValue(descriptor.value, `${path}.${key}`, ancestors),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    }
    return Object.freeze(cloned);
  } finally {
    ancestors.delete(value);
  }
};

const cloneConfigData = (value: unknown, path = "config"): RuntimeConfigData => {
  if (!isRecord(value)) {
    throw new RuntimeConfigUiError("INVALID_CONFIG", `${path} must be an object`);
  }
  return cloneConfigValue(value, path, new WeakSet()) as RuntimeConfigData;
};

const SOURCE_FIELDS = new Set([
  "driverId",
  "displayName",
  "iconKey",
  "availability",
  "schema",
  "config",
]);

const snapshotRuntimeConfigSource = (value: unknown): RuntimeConfigUiSource => {
  if (!isRecord(value)) {
    throw new RuntimeConfigUiError("INVALID_CONFIG", "source must be a plain data object");
  }
  const descriptors = plainDataDescriptors(value, "source");
  const unknownField = Object.keys(descriptors).find((field) => !SOURCE_FIELDS.has(field));
  if (unknownField !== undefined) {
    throw new RuntimeConfigUiError(
      "INVALID_CONFIG",
      `source contains unknown field ${unknownField}`,
    );
  }
  const field = (name: string, required: boolean): unknown => {
    const descriptor = descriptors[name];
    if (descriptor === undefined) {
      if (required) {
        throw new RuntimeConfigUiError("INVALID_CONFIG", `source.${name} is required`);
      }
      return undefined;
    }
    return descriptor.value;
  };
  const driverId = field("driverId", true);
  const displayName = field("displayName", false);
  const iconKey = field("iconKey", false);
  if (typeof driverId !== "string") {
    throw new RuntimeConfigUiError("INVALID_CONFIG", "source.driverId must be a string");
  }
  if (displayName !== undefined && typeof displayName !== "string") {
    throw new RuntimeConfigUiError("INVALID_CONFIG", "source.displayName must be a string");
  }
  if (iconKey !== undefined && typeof iconKey !== "string") {
    throw new RuntimeConfigUiError("INVALID_CONFIG", "source.iconKey must be a string");
  }
  const availabilityValue = cloneConfigValue(
    field("availability", true),
    "source.availability",
    new WeakSet(),
  );
  if (!isRecord(availabilityValue)) {
    throw new RuntimeConfigUiError("INVALID_CONFIG", "source.availability must be an object");
  }
  const availabilityKeys = Object.keys(availabilityValue);
  const availability =
    availabilityValue.status === "available"
      ? (() => {
          if (availabilityKeys.some((key) => key !== "status")) {
            throw new RuntimeConfigUiError(
              "INVALID_CONFIG",
              "available source.availability contains unknown fields",
            );
          }
          return Object.freeze({ status: "available" as const });
        })()
      : (() => {
          if (
            availabilityValue.status !== "unavailable" ||
            (availabilityValue.reason !== "DRIVER_NOT_REGISTERED" &&
              availabilityValue.reason !== "ADAPTER_VERSION_NOT_REGISTERED") ||
            availabilityKeys.some((key) => key !== "status" && key !== "reason")
          ) {
            throw new RuntimeConfigUiError("INVALID_CONFIG", "source.availability is invalid");
          }
          return Object.freeze({
            status: "unavailable" as const,
            reason: availabilityValue.reason,
          });
        })();
  const config = cloneConfigData(field("config", true), "source.config");
  const schemaValue = field("schema", false);
  return Object.freeze({
    driverId,
    ...(displayName === undefined ? {} : { displayName }),
    ...(iconKey === undefined ? {} : { iconKey }),
    availability,
    ...(schemaValue === undefined
      ? {}
      : {
          schema: cloneConfigValue(
            schemaValue,
            "source.schema",
            new WeakSet(),
          ) as unknown as RuntimeConfigSchemaV1,
        }),
    config,
  });
};

/**
 * Passive UI-only boundary. JSON.parse cannot report duplicate object keys, so
 * this source must never grant runtime admission, capability, or authority.
 */
export function decodeRuntimeConfigUiSourceJson(jsonText: string): RuntimeConfigUiSource {
  if (typeof jsonText !== "string") {
    throw new RuntimeConfigUiError("INVALID_SOURCE", "Runtime config source must be JSON text");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(jsonText) as unknown;
  } catch (error) {
    throw new RuntimeConfigUiError(
      "INVALID_SOURCE",
      `Runtime config source is not valid JSON: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
  const source = snapshotRuntimeConfigSource(parsed);
  authenticSources.add(source);
  return source;
}

const nonEmpty = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new RuntimeConfigUiError("INVALID_SCHEMA", `${path} must be a non-empty string`);
  }
  return value;
};

export function validateRuntimeConfigSchema(schema: unknown): RuntimeConfigSchemaV1 {
  schema = cloneConfigValue(schema, "schema", new WeakSet());
  if (!isRecord(schema)) {
    throw new RuntimeConfigUiError("INVALID_SCHEMA", "schema must be an object");
  }
  if (schema.schema !== RUNTIME_CONFIG_UI_SCHEMA) {
    throw new RuntimeConfigUiError(
      "INVALID_SCHEMA",
      `Unsupported runtime config schema ${String(schema.schema)}`,
    );
  }
  const title = nonEmpty(schema.title, "schema.title");
  if (!Array.isArray(schema.sections)) {
    throw new RuntimeConfigUiError("INVALID_SCHEMA", "schema.sections must be an array");
  }
  const sectionIds = new Set<string>();
  const fieldKeys = new Set<string>();
  const sections = schema.sections.map((rawSection, sectionIndex) => {
    if (!isRecord(rawSection)) {
      throw new RuntimeConfigUiError(
        "INVALID_SCHEMA",
        `section[${sectionIndex}] must be an object`,
      );
    }
    const sectionId = nonEmpty(rawSection.id, `section[${sectionIndex}].id`);
    if (!OPEN_ID_PATTERN.test(sectionId)) {
      throw new RuntimeConfigUiError("INVALID_SCHEMA", `Invalid section id ${sectionId}`);
    }
    const sectionLabel = nonEmpty(rawSection.label, `section.${sectionId}.label`);
    if (sectionIds.has(sectionId)) {
      throw new RuntimeConfigUiError("INVALID_SCHEMA", `Duplicate section ${sectionId}`);
    }
    sectionIds.add(sectionId);
    if (!Array.isArray(rawSection.fields)) {
      throw new RuntimeConfigUiError(
        "INVALID_SCHEMA",
        `section.${sectionId}.fields must be an array`,
      );
    }
    const fields = rawSection.fields.map((rawField, fieldIndex) => {
      if (!isRecord(rawField)) {
        throw new RuntimeConfigUiError(
          "INVALID_SCHEMA",
          `section.${sectionId}.fields[${fieldIndex}] must be an object`,
        );
      }
      const key = nonEmpty(rawField.key, `section.${sectionId}.fields[${fieldIndex}].key`);
      if (!FIELD_KEY_PATTERN.test(key)) {
        throw new RuntimeConfigUiError("INVALID_SCHEMA", `Invalid field key ${key}`);
      }
      const label = nonEmpty(rawField.label, `field.${key}.label`);
      if (fieldKeys.has(key)) {
        throw new RuntimeConfigUiError("INVALID_SCHEMA", `Duplicate field ${key}`);
      }
      fieldKeys.add(key);
      const control = rawField.control;
      if (!(["text", "path", "toggle", "select"] as const).includes(control as never)) {
        throw new RuntimeConfigUiError("INVALID_SCHEMA", `Invalid control for ${key}`);
      }
      if (typeof rawField.required !== "boolean") {
        throw new RuntimeConfigUiError("INVALID_SCHEMA", `field.${key}.required must be boolean`);
      }
      let options: RuntimeConfigFieldSchema["options"];
      if (control === "select") {
        if (!Array.isArray(rawField.options) || rawField.options.length === 0) {
          throw new RuntimeConfigUiError(
            "INVALID_SCHEMA",
            `Select field ${key} must declare options`,
          );
        }
        const optionValues = new Set<string>();
        options = rawField.options.map((rawOption, optionIndex) => {
          if (!isRecord(rawOption)) {
            throw new RuntimeConfigUiError(
              "INVALID_SCHEMA",
              `field.${key}.options[${optionIndex}] must be an object`,
            );
          }
          const value = nonEmpty(rawOption.value, `field.${key}.option.value`);
          const optionLabel = nonEmpty(rawOption.label, `field.${key}.option.label`);
          if (optionValues.has(value)) {
            throw new RuntimeConfigUiError(
              "INVALID_SCHEMA",
              `Select field ${key} has duplicate option ${value}`,
            );
          }
          optionValues.add(value);
          return Object.freeze({ value, label: optionLabel });
        });
      } else if (rawField.options !== undefined) {
        throw new RuntimeConfigUiError(
          "INVALID_SCHEMA",
          `Only select fields may declare options (${key})`,
        );
      }
      const description =
        rawField.description === undefined
          ? undefined
          : nonEmpty(rawField.description, `field.${key}.description`);
      return Object.freeze({
        key,
        label,
        ...(description === undefined ? {} : { description }),
        control: control as RuntimeConfigFieldSchema["control"],
        required: rawField.required,
        ...(options === undefined ? {} : { options: Object.freeze([...options]) }),
      });
    });
    const description =
      rawSection.description === undefined
        ? undefined
        : nonEmpty(rawSection.description, `section.${sectionId}.description`);
    return Object.freeze({
      id: sectionId,
      label: sectionLabel,
      ...(description === undefined ? {} : { description }),
      fields: Object.freeze(fields),
    });
  });
  return Object.freeze({
    schema: RUNTIME_CONFIG_UI_SCHEMA,
    title,
    sections: Object.freeze(sections),
  });
}

const fieldValue = (config: RuntimeConfigData, key: string): RuntimeConfigValue | undefined => {
  const value = config[key];
  if (
    value === undefined ||
    value === null ||
    typeof value === "boolean" ||
    typeof value === "number" ||
    typeof value === "string"
  ) {
    return value;
  }
  return undefined;
};

const fieldError = (
  field: RuntimeConfigFieldSchema,
  rawValue: unknown,
): RuntimeConfigFieldViewModel["error"] => {
  if (rawValue === undefined || rawValue === null || rawValue === "") {
    return field.required ? "REQUIRED" : null;
  }
  if (field.control === "toggle") return typeof rawValue === "boolean" ? null : "INVALID_TYPE";
  if (typeof rawValue !== "string") return "INVALID_TYPE";
  if (field.control === "select" && !field.options?.some((option) => option.value === rawValue)) {
    return "INVALID_OPTION";
  }
  return null;
};

export function buildRuntimeConfigViewModel(source: RuntimeConfigUiSource): RuntimeConfigViewModel {
  if (!authenticSources.has(source)) {
    throw new RuntimeConfigUiError(
      "INVALID_SOURCE",
      "Runtime config source must originate from decodeRuntimeConfigUiSourceJson",
    );
  }
  if (!OPEN_ID_PATTERN.test(source.driverId)) {
    throw new RuntimeConfigUiError("INVALID_SCHEMA", `Invalid driver id ${source.driverId}`);
  }
  const displayName =
    typeof source.displayName === "string" && source.displayName.trim().length > 0
      ? source.displayName.trim()
      : source.driverId;
  const available = source.availability.status === "available";
  const iconKey =
    available && typeof source.iconKey === "string" && source.iconKey.trim().length > 0
      ? source.iconKey.trim()
      : GENERIC_RUNTIME_ICON;
  const schema =
    available && source.schema !== undefined
      ? validateRuntimeConfigSchema(source.schema)
      : undefined;
  const editable = available && schema !== undefined;
  const opaqueConfig = cloneConfigData(source.config);
  const availability =
    source.availability.status === "available"
      ? Object.freeze({ status: "available" as const })
      : Object.freeze({
          status: "unavailable" as const,
          reason: source.availability.reason,
        });
  const sections = Object.freeze(
    schema?.sections.map((section) =>
      Object.freeze({
        ...section,
        fields: Object.freeze(
          section.fields.map((field) =>
            Object.freeze({
              ...field,
              id: `runtime-config-${source.driverId}-${section.id}-${field.key}`,
              value: fieldValue(opaqueConfig, field.key),
              error: fieldError(field, opaqueConfig[field.key]),
            }),
          ),
        ),
      }),
    ) ?? [],
  );
  const viewModel = Object.freeze({
    driverId: source.driverId,
    title: schema?.title ?? displayName,
    icon: Object.freeze({
      key: iconKey,
      accessibleLabel: `${displayName} runtime`,
      fallback: iconKey === GENERIC_RUNTIME_ICON,
    }),
    availability,
    editable,
    sections,
    opaqueConfig,
  });
  editPolicies.set(
    viewModel,
    Object.freeze({
      editable,
      knownFields: new Set(
        schema?.sections.flatMap((section) => section.fields.map((field) => field.key)),
      ),
      opaqueConfig,
    }),
  );
  return viewModel;
}

export function applyRuntimeConfigChanges(
  viewModel: RuntimeConfigViewModel,
  changes: RuntimeConfigData,
): RuntimeConfigData {
  const policy = editPolicies.get(viewModel);
  if (policy === undefined) {
    throw new RuntimeConfigUiError(
      "INVALID_VIEW_MODEL",
      "Runtime config edits require a view model created by this module",
    );
  }
  if (!policy.editable) {
    return cloneConfigData(policy.opaqueConfig);
  }
  const next: Record<string, unknown> = { ...policy.opaqueConfig };
  const changeSnapshot = cloneConfigData(changes, "changes");
  for (const [key, value] of Object.entries(changeSnapshot)) {
    if (!policy.knownFields.has(key)) {
      throw new RuntimeConfigUiError("UNKNOWN_FIELD", `Field ${key} is not in the adapter schema`);
    }
    next[key] = cloneConfigValue(value, `changes.${key}`, new WeakSet());
  }
  return cloneConfigData(next);
}
