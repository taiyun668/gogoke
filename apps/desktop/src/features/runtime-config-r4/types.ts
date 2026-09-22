export const RUNTIME_CONFIG_UI_SCHEMA = "gogoke.runtime-config-ui.v1" as const;
export const GENERIC_RUNTIME_ICON = "runtime-generic" as const;

export type RuntimeConfigValue = null | boolean | number | string;

export interface RuntimeConfigOption {
  readonly value: string;
  readonly label: string;
}

export interface RuntimeConfigFieldSchema {
  readonly key: string;
  readonly label: string;
  readonly description?: string;
  readonly control: "text" | "path" | "toggle" | "select";
  readonly required: boolean;
  readonly options?: ReadonlyArray<RuntimeConfigOption>;
}

export interface RuntimeConfigSectionSchema {
  readonly id: string;
  readonly label: string;
  readonly description?: string;
  readonly fields: ReadonlyArray<RuntimeConfigFieldSchema>;
}

export interface RuntimeConfigSchemaV1 {
  readonly schema: typeof RUNTIME_CONFIG_UI_SCHEMA;
  readonly title: string;
  readonly sections: ReadonlyArray<RuntimeConfigSectionSchema>;
}

export type RuntimeConfigData = Readonly<Record<string, unknown>>;

export interface RuntimeConfigUiSource {
  readonly driverId: string;
  readonly displayName?: string;
  readonly iconKey?: string;
  readonly availability:
    | { readonly status: "available" }
    | {
        readonly status: "unavailable";
        readonly reason: "DRIVER_NOT_REGISTERED" | "ADAPTER_VERSION_NOT_REGISTERED";
      };
  readonly schema?: RuntimeConfigSchemaV1;
  readonly config: RuntimeConfigData;
}

export interface RuntimeConfigFieldViewModel extends RuntimeConfigFieldSchema {
  readonly id: string;
  readonly value: RuntimeConfigValue | undefined;
  readonly error: "REQUIRED" | "INVALID_TYPE" | "INVALID_OPTION" | null;
}

export interface RuntimeConfigSectionViewModel {
  readonly id: string;
  readonly label: string;
  readonly description?: string;
  readonly fields: ReadonlyArray<RuntimeConfigFieldViewModel>;
}

export interface RuntimeConfigViewModel {
  readonly driverId: string;
  readonly title: string;
  readonly icon: {
    readonly key: string;
    readonly accessibleLabel: string;
    readonly fallback: boolean;
  };
  readonly availability: RuntimeConfigUiSource["availability"];
  readonly editable: boolean;
  readonly sections: ReadonlyArray<RuntimeConfigSectionViewModel>;
  /** Unknown fields survive edits and downgrade/unavailable states unchanged. */
  readonly opaqueConfig: RuntimeConfigData;
}
