/**
 * Perigee API client.
 *
 * Uses the browser-native Fetch API so the frontend does not need an extra
 * Axios dependency. Set NEXT_PUBLIC_API_URL in production to point at the Rust
 * simulation engine backend; local development defaults to localhost.
 */

import type { AnalyzeResponse } from "./sorobantypes";

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

export const API_URL =
  process.env.NEXT_PUBLIC_API_URL?.replace(/\/+$/, "") ?? DEFAULT_DEV_API_URL;

export const apiConfig = {
  baseUrl: API_URL,
  environment: process.env.NODE_ENV ?? "development",
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
      await sleep(delayMs);
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
