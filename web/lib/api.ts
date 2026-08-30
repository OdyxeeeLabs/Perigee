/**
 * Perigee API client.
 *
 * Uses the browser-native Fetch API so the frontend does not need an extra
 * Axios dependency. In development, NEXT_PUBLIC_API_URL points at the Rust
 * simulation engine backend; in production, requests are proxied through
 * Next.js API routes (pages/api/[[...path]].ts) to avoid CORS issues.
 */

import type { AnalyzeResponse } from "./sorobantypes";
import { trackTelemetryEvent } from "./telemetry";

import {
  AnalyzeRequestDto,
  AnalyzeWasmRequestDto,
  ValidationError as DtoValidationError,
  validateDto,
} from "./dtos";

const DEFAULT_DEV_API_URL = "http://localhost:8080";

export class ValidationError extends DtoValidationError {}

export async function validateAnalyzeRequest(req: AnalyzeRequest): Promise<AnalyzeRequest> {
  return validateDto(AnalyzeRequestDto, req);
}

export async function validateAnalyzeWasmRequest(
  req: AnalyzeWasmRequest,
): Promise<AnalyzeWasmRequest> {
  return validateDto(AnalyzeWasmRequestDto, req);
}

const isProduction = process.env.NODE_ENV === "production";
const configuredUrl = process.env.NEXT_PUBLIC_API_URL?.replace(/\/+$/, "") ?? DEFAULT_DEV_API_URL;

export const API_URL = isProduction ? "/api" : configuredUrl;

export const apiConfig = {
  baseUrl: API_URL,
  environment: process.env.NODE_ENV ?? "development",
  apiVersion: "v1",
};

export interface ApiRequestOptions extends Omit<RequestInit, "body"> {
  params?: Record<string, string | number | boolean | null | undefined>;
  token?: string;
  body?: BodyInit | object | null;
}

export class ApiError extends Error {
  status: number;
  statusText: string;
  body: unknown;

  constructor(status: number, statusText: string, body: unknown) {
    const message =
      typeof body === "object" &&
      body !== null &&
      "message" in body &&
      typeof body.message === "string"
        ? body.message
        : statusText;

    super(`API Error ${status}: ${message}`);
    this.name = "ApiError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

export function apiUrl(
  path: string,
  params?: ApiRequestOptions["params"],
): string {
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  const url = new URL(`${API_URL}${normalizedPath}`);

  if (params) {
    Object.entries(params).forEach(([key, value]) => {
      if (value !== undefined && value !== null) {
        url.searchParams.set(key, String(value));
      }
    });
  }

  return url.toString();
}

function isJsonBody(body: ApiRequestOptions["body"]): body is object {
  return (
    body !== null &&
    body !== undefined &&
    typeof body === "object" &&
    !(typeof Blob !== "undefined" && body instanceof Blob) &&
    !(typeof FormData !== "undefined" && body instanceof FormData) &&
    !(
      typeof URLSearchParams !== "undefined" && body instanceof URLSearchParams
    ) &&
    !(body instanceof ArrayBuffer) &&
    !ArrayBuffer.isView(body)
  );
}

function buildBody(body: ApiRequestOptions["body"]): BodyInit | undefined {
  if (body === null || body === undefined) {
    return undefined;
  }

  return isJsonBody(body) ? JSON.stringify(body) : body;
}

async function parseResponse(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return null;
  }

  const contentType = response.headers.get("content-type") ?? "";
  return contentType.includes("application/json")
    ? response.json()
    : response.text();
}

// --- Retry/backoff for transient RPC failures (Stellar testnet timeouts, 5xx, 429) ---

const RETRYABLE_STATUS_CODES = new Set([408, 429, 500, 502, 503, 504]);
const MAX_RETRY_ATTEMPTS = 4;
const BASE_RETRY_DELAY_MS = 500;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isRetryableStatus(status: number): boolean {
  return RETRYABLE_STATUS_CODES.has(status);
}

function isRetryableNetworkError(error: unknown): boolean {
  // fetch() throws TypeError on network-level failures (timeout, DNS, connection reset, etc.)
  return error instanceof TypeError;
}

async function withRetry<T>(fn: () => Promise<T>): Promise<T> {
  let lastError: unknown;

  for (let attempt = 0; attempt < MAX_RETRY_ATTEMPTS; attempt++) {
    try {
      return await fn();
    } catch (error) {
      lastError = error;

      const isRetryableApiError =
        error instanceof ApiError && isRetryableStatus(error.status);
      const isRetryableNetwork = isRetryableNetworkError(error);
      const isLastAttempt = attempt === MAX_RETRY_ATTEMPTS - 1;

      if ((!isRetryableApiError && !isRetryableNetwork) || isLastAttempt) {
        throw error;
      }

      const delayMs = BASE_RETRY_DELAY_MS * 2 ** attempt;
      // Add full-jitter (0–100 % of the backoff window) to spread thundering-herd
      // retries across time and avoid synchronized retry storms on shared RPC nodes.
      const jitter = Math.random() * delayMs;
      await sleep(delayMs + jitter);
    }
  }

  throw lastError;
}

// --- end retry/backoff ---

async function request<T>(
  endpoint: string,
  options: ApiRequestOptions = {},
): Promise<T> {
  const { params, token, headers, body, ...requestInit } = options;
  const requestHeaders = new Headers(headers);

  if (!requestHeaders.has("Accept")) {
    requestHeaders.set("Accept", "application/json");
  }

  if (!requestHeaders.has("X-API-Version")) {
    requestHeaders.set("X-API-Version", apiConfig.apiVersion);
  }

  if (token && !requestHeaders.has("Authorization")) {
    requestHeaders.set("Authorization", `Bearer ${token}`);
  }

  if (isJsonBody(body) && !requestHeaders.has("Content-Type")) {
    requestHeaders.set("Content-Type", "application/json");
  }

  return withRetry(async () => {
    const response = await fetch(apiUrl(endpoint, params), {
      ...requestInit,
      headers: requestHeaders,
      body: buildBody(body),
    });

    const responseBody = await parseResponse(response);

    if (!response.ok) {
      throw new ApiError(response.status, response.statusText, responseBody);
    }

    return responseBody as T;
  });
}

export const apiClient = {
  request,

  get<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: "GET" });
  },

  post<T>(
    endpoint: string,
    body?: ApiRequestOptions["body"],
    options?: ApiRequestOptions,
  ): Promise<T> {
    return request<T>(endpoint, { ...options, method: "POST", body });
  },

  put<T>(
    endpoint: string,
    body?: ApiRequestOptions["body"],
    options?: ApiRequestOptions,
  ): Promise<T> {
    return request<T>(endpoint, { ...options, method: "PUT", body });
  },

  patch<T>(
    endpoint: string,
    body?: ApiRequestOptions["body"],
    options?: ApiRequestOptions,
  ): Promise<T> {
    return request<T>(endpoint, { ...options, method: "PATCH", body });
  },

  delete<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: "DELETE" });
  },
};

export interface AnalyzeRequest {
  contract_id: string;
  function_name: string;
  args?: string[];
  ledger_overrides?: Record<string, string>;
  protocol_version?: number;
  enable_experimental?: boolean;
}

export interface AnalyzeWasmRequest {
  wasm_bytes: string;
  function_name: string;
  args?: string[];
  protocol_version?: number;
  enable_experimental?: boolean;
}

// ── Manager onboarding types (API-33) ──────────────────────────────────────

export interface RegisterManagerRequest {
  stellar_address: string;
  name: string;
  email?: string;
  kyc_document_ref?: string;
}

export interface ManagerRecord {
  id: string;
  stellar_address: string;
  name: string;
  email: string;
  status: string;
  kyc_document_ref: string;
  notes: string;
  created_at: string;
  updated_at: string;
}

export interface ManagerStatusResponse {
  id: string;
  status: string;
  message: string;
}

export const managerService = {
  async register(req: RegisterManagerRequest): Promise<ManagerRecord> {
    return apiClient.post<ManagerRecord>("/managers/register", req);
  },

  async list(status?: string): Promise<ManagerRecord[]> {
    const params = status ? { status } : undefined;
    return apiClient.get<ManagerRecord[]>("/managers", { params });
  },

  async get(id: string): Promise<ManagerRecord> {
    return apiClient.get<ManagerRecord>(`/managers/${id}`);
  },

  async approve(id: string, notes = ""): Promise<ManagerRecord> {
    return apiClient.post<ManagerRecord>(`/managers/${id}/approve`, { notes });
  },

  async reject(id: string, notes = ""): Promise<ManagerRecord> {
    return apiClient.post<ManagerRecord>(`/managers/${id}/reject`, { notes });
  },

  async checkStatus(stellarAddress: string): Promise<ManagerStatusResponse> {
    return apiClient.get<ManagerStatusResponse>(`/managers/status/${stellarAddress}`);
  },
};

export interface VaultRecord {
  id: string;
  manager_id: string;
  name: string;
  status: string;
  config_json: string;
  version: number;
  idempotency_key: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateVaultRequest {
  manager_id: string;
  name: string;
  status?: string;
  config_json?: string;
  idempotency_key?: string;
}

export interface UpdateVaultRequest {
  version: number;
  name?: string;
  status?: string;
  config_json?: string;
}

export const vaultService = {
  async list(managerId: string, token?: string): Promise<VaultRecord[]> {
    return apiClient.get<VaultRecord[]>("/vaults", {
      params: { manager_id: managerId },
      token,
    });
  },

  async get(id: string, token?: string): Promise<VaultRecord> {
    return apiClient.get<VaultRecord>(`/vaults/${id}`, { token });
  },

  async create(req: CreateVaultRequest, token?: string): Promise<VaultRecord> {
    const vault = await apiClient.post<VaultRecord>("/vaults", req, { token });
    trackTelemetryEvent({ name: "vault_create" });
    return vault;
  },

  async update(id: string, req: UpdateVaultRequest, token?: string): Promise<VaultRecord> {
    return apiClient.patch<VaultRecord>(`/vaults/${id}`, req, { token });
  },
};

export const analyzeService = {
  async analyze(req: AnalyzeRequest, token?: string): Promise<AnalyzeResponse> {
    const validatedRequest = await validateAnalyzeRequest(req);
    return apiClient.post<AnalyzeResponse>("/analyze", validatedRequest, { token });
  },

  async analyzeWasm(
    req: AnalyzeWasmRequest,
    token?: string,
  ): Promise<AnalyzeResponse> {
    const validatedRequest = await validateAnalyzeWasmRequest(req);
    return apiClient.post<AnalyzeResponse>("/analyze/wasm", validatedRequest, { token });
  },
};
