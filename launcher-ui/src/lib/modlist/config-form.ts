import type { ConfigOptionDef, ConfigValue, ModConfigSchema } from "../bindings";

export type FieldKind = "bool" | "number" | "string" | "select" | "readonly";

export type FieldModel = {
  key: string;
  label: string;
  kind: FieldKind;
  value: ConfigValue | null;
  /** Select choices as strings; `selectValue` maps one back to its typed value. */
  options?: string[];
  /** Display text per option, parallel to `options`. */
  labels?: string[];
  /** The schema's typed option values, parallel to `options`. */
  choices?: ConfigValue[];
  description?: string;
  min?: number;
  max?: number;
  step?: number;
};

function kindOf(def: ConfigOptionDef, value: ConfigValue): FieldKind {
  switch (def.option_type) {
    case "checkbox":
      return typeof value === "boolean" ? "bool" : "readonly";
    case "slider":
      return typeof value === "number" ? "number" : "readonly";
    case "cycle":
      if (def.options.length > 0) return def.options.some((o) => o === value) ? "select" : "readonly";
      if (typeof value === "string") return "string";
      return typeof value === "number" ? "number" : "readonly";
    default:
      return "readonly";
  }
}

/** One field per schema option. Unknown types, type mismatches and options with no value are read only. */
export function buildForm(schema: ModConfigSchema, saved: Partial<Record<string, ConfigValue>>): FieldModel[] {
  return schema.options.map((def) => {
    const value = saved[def.id] ?? def.default ?? null;
    const field: FieldModel = {
      key: def.id,
      label: def.label || def.id,
      kind: value === null ? "readonly" : kindOf(def, value),
      value,
    };
    if (def.description) field.description = def.description;
    if (field.kind === "select") {
      field.options = def.options.map(String);
      field.choices = [...def.options];
      field.labels = def.options.map((o, i) => def.labels[i] || String(o));
    }
    if (field.kind === "number") {
      if (def.min !== null) field.min = def.min;
      if (def.max !== null) field.max = def.max;
      if (def.step !== null) field.step = def.step;
    }
    return field;
  });
}

/** Changed, editable fields only; unedited fields are omitted. Outgoing numbers are clamped. */
export function diffValues(
  fields: FieldModel[],
  original: Partial<Record<string, ConfigValue>>,
): Record<string, ConfigValue> {
  const out: Record<string, ConfigValue> = {};
  for (const f of fields) {
    if (f.kind === "readonly" || f.value === null || original[f.key] === f.value) continue;
    out[f.key] = f.kind === "number" && typeof f.value === "number" ? clampNumber(f, f.value) : f.value;
  }
  return out;
}

/** The schema's typed value for a select choice, found by its index in `options`. */
export function selectValue(field: FieldModel, option: string): ConfigValue {
  const i = field.options?.indexOf(option) ?? -1;
  return i >= 0 && field.choices && i < field.choices.length ? field.choices[i] : option;
}

/** Clamps a number field's input to its min and max. */
export function clampNumber(field: FieldModel, n: number): number {
  let out = n;
  if (field.min !== undefined) out = Math.max(field.min, out);
  if (field.max !== undefined) out = Math.min(field.max, out);
  return out;
}
