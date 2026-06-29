/**
 * SoroScope API client.
 *
 * Uses the browser-native Fetch API so the frontend does not need an extra
 * Axios dependency. Set NEXT_PUBLIC_API_URL in production to point at the Rust
 * simulation engine backend; local development defaults to localhost.
 */

const DEFAULT_DEV_API_URL = 'http://localhost:8080';

export const API_URL =
  process.env.NEXT_PUBLIC_API_URL?.replace(/\/+$/, '') ?? DEFAULT_DEV_API_URL;

export const apiConfig = {
  baseUrl: API_URL,
  environment: process.env.NODE_ENV ?? 'development',
};

export interface ApiRequestOptions extends Omit<RequestInit, 'body'> {
  params?: Record<string, string | number | boolean | null | undefined>;
  token?: string;
  body?: BodyInit | Record<string, unknown> | unknown[] | null;
}

export class ApiError extends Error {
  status: number;
  statusText: string;
  body: any;

  constructor(status: number, statusText: string, body: any) {
    const message =
      typeof body === 'object' &&
      body !== null &&
      'message' in body &&
      typeof body.message === 'string'
        ? body.message
        : statusText;

    super(`API Error ${status}: ${message}`);
    this.name = 'ApiError';
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

export function apiUrl(path: string, params?: ApiRequestOptions['params']): string {
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
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

function isJsonBody(body: ApiRequestOptions['body']): body is Record<string, unknown> | unknown[] {
  return (
    body !== null &&
    body !== undefined &&
    typeof body === 'object' &&
    !(typeof Blob !== 'undefined' && body instanceof Blob) &&
    !(typeof FormData !== 'undefined' && body instanceof FormData) &&
    !(typeof URLSearchParams !== 'undefined' && body instanceof URLSearchParams) &&
    !(body instanceof ArrayBuffer) &&
    !ArrayBuffer.isView(body)
  );
}

function buildBody(body: ApiRequestOptions['body']): BodyInit | undefined {
  if (body === null || body === undefined) {
    return undefined;
  }

  return isJsonBody(body) ? JSON.stringify(body) : body;
}

async function parseResponse(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return null;
  }

  const contentType = response.headers.get('content-type') ?? '';
  return contentType.includes('application/json')
    ? response.json()
    : response.text();
}

async function request<T>(endpoint: string, options: ApiRequestOptions = {}): Promise<T> {
  const { params, token, headers, body, ...requestInit } = options;
  const requestHeaders = new Headers(headers);

  if (!requestHeaders.has('Accept')) {
    requestHeaders.set('Accept', 'application/json');
  }

  if (token && !requestHeaders.has('Authorization')) {
    requestHeaders.set('Authorization', `Bearer ${token}`);
  }

  if (isJsonBody(body) && !requestHeaders.has('Content-Type')) {
    requestHeaders.set('Content-Type', 'application/json');
  }

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
}

export const apiClient = {
  request,

  get<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'GET' });
  },

  post<T>(endpoint: string, body?: ApiRequestOptions['body'], options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'POST', body });
  },

  put<T>(endpoint: string, body?: ApiRequestOptions['body'], options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'PUT', body });
  },

  patch<T>(endpoint: string, body?: ApiRequestOptions['body'], options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'PATCH', body });
  },

  delete<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'DELETE' });
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
  analyze<T = any>(req: AnalyzeRequest, token?: string): Promise<T> {
    return apiClient.post<T>('/analyze', req, { token });
  },

  analyzeWasm<T = any>(req: AnalyzeWasmRequest, token?: string): Promise<T> {
    return apiClient.post<T>('/analyze/wasm', req, { token });
  },
};
