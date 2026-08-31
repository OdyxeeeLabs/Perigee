/**
 * Perigee API Client
 Production-grade, lightweight, type-safe api client configuration using native Fetch API.
 Integrates Next.js frontend to the Rust Axum backend.
*/

export interface ApiRequestOptions extends RequestInit {
  params?: Record<string, string>;
  token?: string;
}

export class ApiError extends Error {
  status: number;
  statusText: string;
  body: any;

  constructor(status: number, statusText: string, body: any) {
    super(`API Error ${status}: ${body?.message || statusText}`);
    this.name = 'ApiError';
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

const BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';

// ------------------------------------------------------------------------------
// JWT token management (supports key rotation and revocation)
// ------------------------------------------------------------------------------
let accessToken: string | null = null;
let refreshToken: string | null = null;
let refreshPromise: Promise<string | null> | null = null;

/**
 * Store the current access token and refresh token.
 */
export function setAuthTokens(newAccessToken: string, newRefreshToken: string): void {
  accessToken = newAccessToken;
  refreshToken = newRefreshToken;
}

/**
 * Clear all stored tokens (e.g., on logout or revocation).
 */
export function clearAuthTokens(): void {
  accessToken = null;
  refreshToken = null;
}

/**
 * Get the current access token.
 */
export function getAccessToken(): string | null {
  return accessToken;
}

interface RefreshResponse {
  access_token: string;
  refresh_token?: string;
  token_type?: string;
}

/**
 * Attempt to refresh the access token using the stored refresh token.
 * Uses a shared promise to avoid concurrent refresh calls.
 */
async function refreshAccessToken(): Promise<string | null> {
  if (!refreshToken) return null;
  if (refreshPromise) return refreshPromise;

  refreshPromise = (async () => {
    try {
      const response = await fetch(`${BASE_URL}/auth/refresh`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refresh_token: refreshToken }),
      });

      if (!response.ok) {
        clearAuthTokens();
        return null;
      }

      const data = (await response.json()) as RefreshResponse;
      accessToken = data.access_token;
      if (data.refresh_token) {
        refreshToken = data.refresh_token;
      }
      return accessToken;
    } catch {
      clearAuthTokens();
      return null;
    } finally {
      refreshPromise = null;
    }
  })();

  return refreshPromise;
}

/**
 * Revoke the current refresh token on the server and clear local auth state.
 */
export async function revokeToken(): Promise<void> {
  const currentRefreshToken = refreshToken;
  try {
    if (currentRefreshToken) {
      await fetch(`${BASE_URL}/auth/revoke`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refresh_token: currentRefreshToken }),
      });
    }
  } finally {
    clearAuthTokens();
  }
}

// ------------------------------------------------------------------------------
// Core request helper
// ------------------------------------------------------------------------------
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

  const authToken = token || accessToken;
  const defaultHeaders: Record<string, string> = {
    'Content-Type': 'application/json',
    'Accept': 'application/json',
  };

  if (authToken) {
    defaultHeaders['Authorization'] = `Bearer ${authToken}`;
  }

  const buildConfig = (authTokenOverride?: string): RequestInit =>({
    method: options.method || 'GET',
    headers: {
      ...defaultHeaders,
      ...headers,
      ...(authTokenOverride ? { Authorization: `Bearer ${authTokenOverride}` } : {}),
    },
    ...customConfig,
  });

  let response = await fetch(fullUrl, buildConfig());

  if (response.status === 401 && !token && refreshToken) {
    // Access token may be expired or invalid (e.g., due to key rotation).
    // Try to refresh once and retry the request.
    const newAccessToken = await refreshAccessToken();
    if (newAccessToken) {
      response = await fetch(fullUrl, buildConfig(newAccessToken));
    }
  }

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
}

// Typed base request methods
export const apiClient = {
  get<T>(endpoint: string, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, { ...options, method: 'GET' });
  },

  post<T>(endpoint: string, body?: any, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, {
      ...options,
      method: 'POST',
      body: body ? JSON.stringify(body) : undefined,
    });
  },

  put<T>(endpoint: string, body?: any, options?: ApiRequestOptions): Promise<T> {
    return request<T>(endpoint, {
      ...options,
      method: 'PUT',
      body: body ? JSON.stringify(body) : undefined,
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
  async analyze(req: AnalyzeRequest, token?: string): Promise<any> {
    return apiClient.post<any>('/analyze', req, { token });
  },

  /**
   * Analyze custom WASM file binary bytes
   * @param req The WASM bytes analysis request payload
   * @param token JWT authorization token (optional)
   */
  async analyzeWasm(req: AnalyzeWasmRequest, token?: string): Promise<any> {
    return apiClient.post<any>('/analyze/wasm', req, { token });
  },
};

/**
 * Base URL of the Perigee analyzer backend.
 *
 * Reads from NEXT_PUBLIC_API_URL (baked in at build time) and falls z to
 * localhost for local development, so no env file is needed to run locally.
 * In production, set NEXT_PUBLIC_API_URL to the deployed backend's URL.
 */
export const API_URL =
  process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080';

/** Build a full backend URL from a path, e.g. apiUrl('/analyze'). */
export function apiUrl(path: string): string {
  return `${API_URL}${path.startsWith('/') ? path : `/${path}`};
}
