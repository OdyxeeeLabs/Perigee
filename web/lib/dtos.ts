export class ValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ValidationError";
  }
}

export class AnalyzeRequestDto {
  constructor(payload: Record<string, unknown> = {}) {
    this.contract_id = payload.contract_id as string | undefined;
    this.function_name = payload.function_name as string | undefined;
    this.args = payload.args as string[] | undefined;
    this.ledger_overrides = payload.ledger_overrides as Record<string, string> | undefined;
    this.protocol_version = payload.protocol_version as number | undefined;
    this.enable_experimental = payload.enable_experimental as boolean | undefined;
  }

  contract_id?: string;
  function_name?: string;
  args?: string[];
  ledger_overrides?: Record<string, string>;
  protocol_version?: number;
  enable_experimental?: boolean;
}

export class AnalyzeWasmRequestDto {
  constructor(payload: Record<string, unknown> = {}) {
    this.wasm_bytes = payload.wasm_bytes as string | undefined;
    this.function_name = payload.function_name as string | undefined;
    this.args = payload.args as string[] | undefined;
    this.protocol_version = payload.protocol_version as number | undefined;
    this.enable_experimental = payload.enable_experimental as boolean | undefined;
  }

  wasm_bytes?: string;
  function_name?: string;
  args?: string[];
  protocol_version?: number;
  enable_experimental?: boolean;
}

function ensureString(value: unknown, fieldName: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new ValidationError(`${fieldName} must be a non-empty string`);
  }

  return value.trim();
}

function ensureOptionalStringArray(value: unknown, fieldName: string): string[] | undefined {
  if (value === undefined) {
    return undefined;
  }

  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    throw new ValidationError(`${fieldName} must be an array of strings`);
  }

  return value.map((item) => item.trim()).filter(Boolean);
}

function ensureOptionalRecord(value: unknown, fieldName: string): Record<string, string> | undefined {
  if (value === undefined) {
    return undefined;
  }

  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new ValidationError(`${fieldName} must be an object`);
  }

  const normalized: Record<string, string> = {};

  for (const [key, entryValue] of Object.entries(value)) {
    if (typeof entryValue !== "string") {
      throw new ValidationError(`${fieldName}.${key} must be a string`);
    }

    normalized[key] = entryValue;
  }

  return normalized;
}

function ensureOptionalNumber(value: unknown, fieldName: string): number | undefined {
  if (value === undefined) {
    return undefined;
  }

  if (typeof value !== "number" || Number.isNaN(value)) {
    throw new ValidationError(`${fieldName} must be a number`);
  }

  return value;
}

function ensureOptionalBoolean(value: unknown, fieldName: string): boolean | undefined {
  if (value === undefined) {
    return undefined;
  }

  if (typeof value !== "boolean") {
    throw new ValidationError(`${fieldName} must be a boolean`);
  }

  return value;
}

export async function validateDto<T, D extends AnalyzeRequestDto | AnalyzeWasmRequestDto>(
  dtoClass: new (payload: Record<string, unknown>) => D,
  payload: T,
): Promise<D> {
  const dto = new dtoClass(payload as Record<string, unknown>);

  if (dto instanceof AnalyzeRequestDto) {
    return new AnalyzeRequestDto({
      contract_id: ensureString(dto.contract_id, "contract_id"),
      function_name: ensureString(dto.function_name, "function_name"),
      args: ensureOptionalStringArray(dto.args, "args"),
      ledger_overrides: ensureOptionalRecord(dto.ledger_overrides, "ledger_overrides"),
      protocol_version: ensureOptionalNumber(dto.protocol_version, "protocol_version"),
      enable_experimental: ensureOptionalBoolean(dto.enable_experimental, "enable_experimental"),
    }) as D;
  }

  return new AnalyzeWasmRequestDto({
    wasm_bytes: ensureString(dto.wasm_bytes, "wasm_bytes"),
    function_name: ensureString(dto.function_name, "function_name"),
    args: ensureOptionalStringArray(dto.args, "args"),
    protocol_version: ensureOptionalNumber(dto.protocol_version, "protocol_version"),
    enable_experimental: ensureOptionalBoolean(dto.enable_experimental, "enable_experimental"),
  }) as D;
}
