/**
 * SoroScope API Client
 * Production-grade, lightweight, type-safe API client configuration using native Fetch API.
 * Integrates Next.js frontend to the Rust Axum backend.
 */

export interface ApiRequestOptions extends RequestInit {
  params?: Record<string, string>;
  token?: string;
}

export class ApiError extends Error {
  status: number;
  statusText: string;
  body: unknown;

  constructor(status: number, statusText: string, body: unknown) {
    super(`API Error ${status}: ${body?.message || statusText}`);
    this.name = 'ApiError';
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

const BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';

async function request<T>(endpoint: string, options: ApiRequestOptions = {}): Promise<T> {
  const { params, token, headers, ...customConfig } = options;
  
  // Build full query string if params are provided
  let queryString = '';
  if (params) {
    const searchParams = new URLSearchParams();
    Object.entries(params).forEach(([key, val]) => {
      if (val !== undefined && val !== null) {
        searchParams.append(key, val);
      }
    });
    queryString = `?${searchParams.toString()}`;
  }

  const fullUrl = `${BASE_URL}${endpoint}${queryString}`;

  const defaultHeaders: Record<string, string> = {
    'Content-Type': 'application/json',
    'Accept': 'application/json',
  };

  if (token) {
    defaultHeaders['Authorization'] = `Bearer ${token}`;
  }

  const config: RequestInit = {
    method: options.method || 'GET',
    headers: {
      ...defaultHeaders,
      ...headers,
    },
    ...customConfig,
  };

  try {
    const response = await fetch(fullUrl, config);
    
    let responseData: any = null;
    const contentType = response.headers.get('content-type');
    if (contentType && contentType.includes('application/json')) {
      responseData = await response.json();
    } else {
      responseData = await response.text();
    }

    if (!response.ok) {
      throw new ApiError(response.status, response.statusText, responseData);
    }

    return responseData as T;
  } catch (error) {
    if (error instanceof ApiError) {
      throw error;
    }
    // Network errors or aborts
    throw new Error(error instanceof Error ? error.message : 'Network request failed');
  }
}

// Typed base request methods
export const apiClient = {
  get<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'GET' });
  },

  post<T>(endpoint: string, body?: unknown, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, {
      ...options,
      method: 'POST',
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  },

  put<T>(endpoint: string, body?: unknown, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, {
      ...options,
      method: 'PUT',
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  },

  delete<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'DELETE' });
  },
};

// Domain-specific Analyze Service endpoints
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
  /**
   * Profiling a contract invocation by ID
   * @param req The contract analysis request payload
   * @param token JWT authorization token (optional)
   */
  async analyze(req: AnalyzeRequest, token?: string): Promise<unknown> {
    return apiClient.post<unknown>('/analyze', req, { token });
  },

  /**
   * Analyze custom WASM file binary bytes
   * @param req The WASM bytes analysis request payload
   * @param token JWT authorization token (optional)
   */
  async analyzeWasm(req: AnalyzeWasmRequest, token?: string): Promise<unknown> {
    return apiClient.post<unknown>('/analyze/wasm', req, { token });
  },
};

/**
 * Base URL of the SoroScope analyzer backend.
 *
 * Reads from NEXT_PUBLIC_API_URL (baked in at build time) and falls back to
 * localhost for local development, so no env file is needed to run locally.
 * In production, set NEXT_PUBLIC_API_URL to the deployed backend's URL.
 */
export const API_URL =
  process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080';

/** Build a full backend URL from a path, e.g. apiUrl('/analyze'). */
export function apiUrl(path: string): string {
  return `${API_URL}${path.startsWith('/') ? path : `/${path}`}`;
}
