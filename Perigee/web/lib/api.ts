/**
 * Perigee API Client
 * Production-grade, lightweight, type-safe api client configuration using native Fetch API.
 * Integrates Next.js frontend to the Rust Axum backend.
*/

/**
 * Perigee API Client
 * Production-grade, lightweight, type-safe api client configuration using native Fetch API.
 * Integrates Next.js frontend to the Rust Axum backend.
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

// ------------------------------------------------------------------------------
// Development proxy (see dev.proxy.json)
//
// When NEXT_PUBLIC_DEV_PROXY is enabled, requests are routed through a local
// development proxy that can mock API responses (MSW), simulate contracts
// (Anvil/Hardhat forks), inject network conditions (latency, packet loss,
// throttling), log traffic, and replay scenarios. It is zero-config for new
// developers: if dev.proxy.json is absent or the flag is off, the client
// behaves exactly as before.
// ------------------------------------------------------------------------------

export interface DevProxyConfig {
  enabled: boolean;
  url: string;
  latencyMs?: number;
  packetLoss?: number;
  throttleKbps?: number;
  logRequests?: boolean;
  scenario?: string;
}

const DEV_PROXY_DEFAULTS: DevProxyConfig = {
  enabled: false,
  url: 'http://localhost:8787',
  logRequests: true,
};

let devProxyConfig: DevProxyConfig | null = null;

/**
 * Resolve the development proxy configuration.
 *
 * Priority:
 *   1. Explicit runtime config set via `configureDevProxy`.
 *   2. `NEXT_PUBLIC_DEV_PROXY_URL` env var (proxy URL, implies enabled).
 *   3. Built-in defaults (disabled).
 */
export function getDevProxyConfig(): DevProxyConfig {
  if (devProxyConfig) return devProxyConfig;

  const envUrl = process.env.NEXT_PUBLIC_DEV_PROXY_URL;
  devProxyConfig = envUrl
    ? { ...DEV_PROXY_DEFAULTS, enabled: true, url: envUrl }
    : { ...DEV_PROXY_DEFAULTS };

  return devProxyConfig;
}

/**
 * Override the development proxy configuration at runtime. Intended for
 * tests and for wiring values loaded from dev.proxy.json.
 */
export function configureDevProxy(config: Partial<DevProxyConfig>): void {
  devProxyConfig = { ...getDevProxyConfig(), ...config };
}

/**
 * Reset the development proxy configuration to its resolved defaults.
 */
export function resetDevProxyConfig(): void {
  devProxyConfig = null;
}

/**
 * Apply network condition simulation headers so the proxy can emulate
 * latency, packet loss, and bandwidth throttling.
 */
function applyDevProxyHeaders(
  headers: Record<string, string>,
  config: DevProxyConfig,
): void {
  if (config.latencyMs !== undefined) {
    headers['X-Dev-Proxy-Latency'] = String(config.latencyMs);
  }
  if (config.packetLoss !== undefined) {
    headers['X-Dev-Proxy-Packet-Loss'] = String(config.packetLoss);
  }
  if (config.throttleKbps !== undefined) {
    headers['X-Dev-Proxy-Throttle-Kbps'] = String(config.throttleKbps);
  }
  if (config.scenario) {
    headers['X-Dev-Proxy-Scenario'] = config.scenario;
  }
}

/**
 * Resolve the effective base URL, routing through the dev proxy when enabled.
 */
function resolveBaseUrl(): string {
  const config = getDevProxyConfig();
  return config.enabled ? config.url : BASE_URL;
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
      const response = await fetch(`${resolveBaseUrl()}/auth/refresh`, {
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
      await fetch(`${resolveBaseUrl()}/auth/revoke`, {
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

  const proxyConfig = getDevProxyConfig();
  const fullUrl = `${resolveBaseUrl()}${endpoint}${queryString}`;

  const authToken = token || accessToken;
  const defaultHeaders: Record<string, string> = {
    'Content-Type': 'application/json',
    'Accept': 'application/json',
  };

  if (authToken) {
    defaultHeaders['Authorization'] = `Bearer ${authToken}`;
  }

  if (proxyConfig.enabled) {
    applyDevProxyHeaders(defaultHeaders, proxyConfig);
    if (proxyConfig.logRequests) {
      // eslint-disable-next-line no-console
      console.debug(`[dev-proxy] ${options.method || 'GET'} ${fullUrl}`);
    }
  }

  const buildConfig = (authTokenOverride?: string): RequestInit => ({
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
