import { createServer, type IncomingMessage, type Server, type ServerResponse } from "http";
import { createServer as createHttpsServer, type ServerOptions } from "https";
import { promises as fs } from "fs";
import path from "path";
import { spawnSync, type ChildProcess } from "child_process";
import { fileURLToPath } from "url";

// -----------------------------------------------------------------------------
// Types
// -----------------------------------------------------------------------------

export interface MockRule {
  /** HTTP method to match, e.g. "GET", "POST", or "*". */
  method?: string;
  /** Regex or string pattern matched against the request path + query. */
  path: string;
  /** HTTP status code to return. */
  status?: number;
  /** Response headers. */
  headers?: Record<string, string>;
  /** JSON or string response body. */
  body?: unknown;
  /** Optional delay before responding, in milliseconds. */
  delayMs?: number;
  /** Optional number of times to apply this rule. */
  times?: number;
}

export interface NetworkConditions {
  /** Base latency added to every response, in milliseconds. */
  latencyMs?: number;
  /** Random latency jitter, in milliseconds. */
  jitterMs?: number;
  /** Probability [0-1] that a request is dropped. */
  packetLoss?: number;
  /** Throttle response body to this many KiB/s. 0 disables. */
  throttleKbps?: number;
  /** Probability [0-1] that a request fails with failureStatus. */
  failureRate?: number;
  /** Status code used when failureRate triggers. */
  failureStatus?: number;
}

export interface LoggingConfig {
  enabled?: boolean;
  /** Whether to capture request/response bodies. */
  body?: boolean;
  /** Maximum body bytes to capture. */
  maxBodyBytes?: number;
  /** File to append JSON-lines logs to. */
  file?: string;
}

export interface ForkConfig {
  enabled?: boolean;
  /** "anvan" or "hardhat". */
  tool?: "anvan" | "hardhat";
  /** Chain identifier, e.g. "stellar", "ethereum". */
  chain?: string;
  /** Remote RPC URL to fork from. */
  forkUrl?: string;
  /** Local RPC URL the fork listens on. */
  rpc?: string;
  /** Number of accounts to pre-fund. */
  accounts?: number;
  /** Auto-mine block interval in ms. 0 disables. */
  blockTimeMs?: number;
  /** Extra CLI args forwarded to the fork tool. */
  extraArgs?: string[];
}

export interface ScenarioStep {
  /** Name of the scenario this step belongs to. */
  scenario?: string;
  /** Request matcher. */
  match: MockRule;
  /** Optional delay before applying. */
  delayMs?: number;
}

export interface DevProxyConfig {
  enabled?: boolean;
  port?: number;
  /** UpStream target backend URL. */
  target?: string;
  logging?: LoggingConfig;
  network?: NetworkConditions;
  mocks?: MockRule[];
  scenarios?: ScenarioStep[];
  fork?: ForkConfig;
}

export interface RequestLogEntry {
  id: string;
  timestamp: string;
  method: string;
  url: string;
  status: number;
  durationMs: number;
  mocked: boolean;
  scenario?: string;
  requestBody?: string;
  responseBody?: string;
}

// -----------------------------------------------------------------------------
// Defaults + config loading
// -----------------------------------------------------------------------------

export const DEFAULT_CONFIG: Required<DevProxyConfig> = {
  enabled: true,
  port: 8787,
  target: "http://localhost:8080",
  logging: {
    enabled: true,
    body: true,
    maxBodyBytes: 32768,
    file: ".dev-proxy/requests.log.jsonl",
  },
  network: {
    latencyMs: 0,
    jitterMs: 0,
    packetLoss: 0,
    throttleKbps: 0,
    failureRate: 0,
    failureStatus: 503,
  },
  mocks: [],
  scenarios: [],
  fork: {
    enabled: false,
    tool: "anvan",
    chain: "stellar",
    rpc: "http://127.0.0.1:8545",
    forkUrl: "",
    accounts: 10,
    blockTimeMs: 0,
  },
};

export function defaultConfigPath(): string {
  const here = path.dirname(fileURLToPath(import.meta.url));
  return path.resolve(here, "..", "dev.proxy.json");
}

function mergeConfig(partial: DevProxyConfig): Required<DevProxyConfig> {
  return {
    ...DEFAULT_CONFIG,
    ...partial,
    logging: { ...DEFAULT_CONFIG.logging, ...(partial.logging ?? {}) },
    network: { ...DEFAULT_CONFIG.network, ...(partial.network ?? {}) },
    fork: { ...DEFAULT_CONFIG.fork, ...(partial.fork ?? {}) },
    mocks: partial.mocks ?? [],
    scenarios: partial.scenarios ?? [],
  };
}

export async function loadConfig(configPath?: string): Promise<Required<DevProxyConfig>> {
  const file = configPath ?? defaultConfigPath();
  try {
    const raw = await fs.readFile(file, "utf-8");
    const parsed = JSON.parse(raw) as DevProxyConfig;
    return mergeConfig(parsed);
  } catch (err) {
    const e = err as NodeJSError & { code?: string };
    if (e.code === "ENOENT") {
      return { ...DEFAULT_CONFIG };
    }
    throw err;
  }
}

// -----------------------------------------------------------------------------
// Matching + network simulation
// -----------------------------------------------------------------------------

function compilePattern(pattern: string): RegExp {
  if (pattern.startsWith("/") && pattern.endsWith("/") && pattern.length > 2) {
    return new RegExp(pattern.slice(1, -1));
  }
  const escaped = pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/\\*/g, ".*");
  return new RegExp(`^${escaped}$`);
}

export function matchRule(rule: MockRule, method: string, url: string): boolean {
  if (rule.method && rule.method !== "*" && rule.method.toUpperCase() !== method.toUpperCase()) {
    return false;
  }
  return compilePattern(rule.path).test(url);
}

export function computeDelay(network: NetworkConditions): number {
  const base = Math.max(0, network.latencyMs ?? 0);
  const jitter = Math.max(0, network.jitterMs ?? 0);
  if (jitter === 0) return base;
  return base + Math.round((Math.random() * 2 - 1) * jitter);
}

export function shouldDrop(network: NetworkConditions): boolean {
  const loss = network.packetLoss ?? 0;
  if (loss <= 0) return false;
  return Math.random() < loss;
}

export function shouldFail(network: NetworkConditions): boolean {
  const rate = network.failureRate ?? 0;
  if (rate <= 0) return false;
  return Math.random() < rate;
}

// -----------------------------------------------------------------------------
// Logging
// -----------------------------------------------------------------------------

export class RequestLogger {
  private readonly config: LoggingConfig;
  private readonly entries: RequestLogEntry[] = [];
  private writeChain: Promise<void> = Promise.resolve();

  constructor(config: LoggingConfig) {
    this.config = config;
  }

  get all(): RequestLogEntry[] {
    return this.entries;
  }

  async record(entry: RequestLogEntry): Promise<void> {
    this.entries.push(entry);
    if (!this.config.enabled) return;
    const line = JSON.stringify(entry) + "\n";
    this.writeChain = this.writeChain.then(async () => {
      if (!this.config.file) return;
      const file = path.resolve(process.cwd(), this.config.file);
      await fs.mkdir(path.dirname(file), { recursive: true });
      await fs.appendFile(file, line, "utf-8");
    }).catch(() => undefined);
    await this.writeChain;
  }

  truncate(body: unknown): string | undefined {
    if (!this.config.body || body === undefined) return undefined;
    const text = typeof body === "string" ? body : JSON.stringify(body);
    const max = this.config.maxBodyBytes ?? 32768;
    return text.length > max ? text.slice(0, max) : text;
  }
}

// -----------------------------------------------------------------------------
// Fork manager (Anvan / Hardhat)
// -----------------------------------------------------------------------------

export interface ForkHandle {
  url: string;
  tool: "anvan" | "hardhat";
  stop(): void;
}

function buildForkArgs(config: Required<ForkConfig>): string[] {
  if (config.tool === "hardhat") {
    const args = ["node", "--hostname", "127.0.0.1", "--port", String(new URL(config.rpc).port || 8545)];
    if (config.forkUrl) args.push("--fork", config.forkUrl);
    if (config.blockTimeMs > 0) args.push("--interval", String(Math.round(config.blockTimeMs / 1000)));
    return [...args, ...(config.extraArgs ?? [])];
  }
  const port = String(new URL(config.rpc).port || 8545);
  const args = ["node", "--host", "127.0.0.1", "--port", port];
  if (config.forkUrl) args.push("--fork-url", config.forkUrl);
  if (config.accounts > 0) args.push("--accounts", String(config.accounts));
  if (config.blockTimeMs > 0) args.push("--block-time", String(Math.round(config.blockTimeMs / 1000)));
  return [...args, ...(config.extraArgs ?? [])];
}

function waitForRPC(url: string, timeoutMs = 15000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve, reject) => {
    const tick = async () => {
      try {
        const res = await fetch(url, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_chainId", params: [] }),
        });
        if (res.ok) return resolve();
      } catch {
        // not ready yet
      }
      if (Date.now() > deadline) return reject(new Error(`RPC at ${url} did not become ready`));
      setTimeout(tick, 250);
    };
    void tick();
  });
}

export async function startFork(config: Required<ForkConfig>): Promise<ForkHandle> {
  const args = buildForkArgs(config);
  const bin = args.shift()!;
  const child: ChildProcess = spawnSync(bin, args, { stdio: "inherit" });
  await waitForRPC(config.rpc);
  return {
    url: config.rpc,
    tool: config.tool,
    stop: () => {
      if (!child.killed) child.kill();
    },
  };
}

// -----------------------------------------------------------------------------
// Proxy core
// -----------------------------------------------------------------------------

export interface ProxyOptions {
  config: Required<DevProxyConfig>;
  logger?: RequestLogger;
}

export interface DevProxy {
  server: Server;
  port: number;
  close(): Promise<void>;
  logger: RequestLogger;
  fork?: ForkHandle;
}

function readBody(req: IncomingMessage, maxBytes: number): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    req.on("data", (chunk: Buffer) => {
      total += chunk.length;
      if (total > maxBytes) {
        reject(new Error("Request body too large"));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => resolve(Buffer.concat(chunks)));
    req.on("error", reject);
  });
}

function sendJson(res: ServerResponse, status: number, body: unknown, headers?: Record<string, string>): void {
  const payload = typeof body === "string" ? body : JSON.stringify(body ?? null);
  res.writeHead({
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
    ...headers,
  });
  res.statusCode = status;
  res.end(payload);
}

function sleep(ms: number): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function forwardToTarget(
  target: string,
  method: string,
  url: string,
  headers: Record<string, string | string[] | undefined>,
  body: Buffer,
): Promise<{ status: number; headers: Record<string, string>; body: Buffer }> {
  const targetUrl = new URL(url, target);
  const res = await fetch(targetUrl, {
    method,
    headers: headers as Record<string, string>,
    body: body.length > 0 ? body : undefined,
  });
  const buf = Buffer.from(await res.arrayBuffer());
  const outHeaders: Record<string, string> = {};
  res.headers.forEach((v, k) => {
    outHeaders[k] = v;
  });
  return { status: res.status, headers: outHeaders, body: buf };
}

function throttledSend(res: ServerResponse, body: Buffer, kbps: number): Promise<void> {
  const bytesPerSecond = Math.max(1, Math.round(kbps * 1024));
  const chunkSize = Math.max(1, Math.round(bytesPerSecond / 20));nd(bytesPerSecond / 20));
  let offset = 0;
  while (offset < body.length) {
    const chunk = body.subarray(offset, offset + chunkSize);
    offset += chunk.length;
    res.write(chunk);
    await sleep((chunk.length / bytesPerSecond) * 1000);
  }
  res.end();
}

export async function createDevProxy(options: ProxyOptions): Promise<DevProxy> {
  const { config } = options;
  const logger = options.logger ?? new RequestLogger(config.logging);
  const mockUsage = new Map<MockRule, number>();

  const handler = async (req: IncomingMessage, res: ServerResponse) => {
    const started = Date.now();
    const method = req.method ?? "GET";
    const url = req.url ?? "/";
    const id = Math.random().toString(36).slice(2, 10);
    const network = config.network;

    let requestBody = Buffer.alloc(0);
    try {
      requestBody = await readBody(req, logger.configMaxBodyBytes());
    } catch {
      sendJson(res, 413, { error: "request too large" });
      return;
    }

    // Network conditions: packet loss.
    if (shouldDrop(network)) {
      req.socket.destroy();
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status: 0,
        durationMs: Date.now() - started,
        mocked: false,
        requestBody: logger.truncate(requestBody.toString()),
      });
      return;
    }

    // Network conditions: failure injection.
    if (shouldFail(network)) {
      await sleep(computeDelay(network));
      sendJson(res, network.failureStatus ?? 503, { error: "injected failure" });
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status: network.failureStatus ?? 503,
        durationMs: Date.now() - started,
        mocked: false,
        requestBody: logger.truncate(requestBody.toString()),
      });
      return;
    }

    // Scenario replay: first matching step wins.
    const scenarioStep = config.scenarios.find((s) => matchRule(s.match, method, url));
    if (scenarioStep) {
      const rule = scenarioStep.match;
      const delay = computeDelay(network) + (scenarioStep.delayMs ?? rule.delayMs ?? 0);
      await sleep(delay);
      const status = rule.status ?? 200;
      const body = rule.body ?? { ok: true, scenario: scenarioStep.scenario };
      sendJson(res, status, body, rule.headers);
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status,
        durationMs: Date.now() - started,
        mocked: true,
        scenario: scenarioStep.scenario,
        requestBody: logger.truncate(requestBody.toString()),
        responseBody: logger.truncate(body),
      });
      return;
    }

    // Mock matching.
    const mock = config.mocks.find((r) => {
      if (!typeof r.times === "number") return true;
      const used = mockUsage.get(r) ?? 0;
      return used < r.times!;
    } && ((r) => matchRule(r, method, url)));
    if (mock) {
      mockUsage.set(mock, (mockUsage.get(mock) ?? 0) + 1);
      const delay = computeDelay(network) + (mock.delayMs ?? 0);
      await sleep(delay);
      const status = mock.status ?? 200;
      const body = mock.body ?? { ok: true };
      sendJson(res, status, body, mock.headers);
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status,
        durationMs: Date.now() - started,
        mocked: true,
        requestBody: logger.truncate(requestBody.toString()),
        responseBody: logger.truncate(body),
      });
      return;
    }

    // Forward to the real backend.
    try {
      const delay = computeDelay(network);
      await sleep(delay);
      const forwarded = await forwardToTarget(
        config.target,
        method,
        url,
        req.headers as Record<string, string>,
        requestBody,
      );
      const throttle = network.throttleKbps ?? 0;
      res.writeHead(forwarded.headers);
      res.statusCode = forwarded.status;
      if (throttle > 0) {
        await throttledSend(res, forwarded.body, throttle);
      } else {
        res.end(forwarded.body);
      }
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status: forwarded.status,
        durationMs: Date.now() - started,
        mocked: false,
        requestBody: logger.truncate(requestBody.toString()),
        responseBody: logger.truncate(forwarded.body.toString()),
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : "upstream error";
      sendJson(res, 502, { error: message });
      await logger.record({
        id,
        timestamp: new Date().toISOString(),
        method,
        url,
        status: 502,
        durationMs: Date.now() - started,
        mocked: false,
        requestBody: logger.truncate(requestBody.toString()),
      });
    }
  };

  const server = createServer((req, res) => {
    void handler(req, res);
  });

  const listen = () =>
    new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(config.port, "127.0.0.1", () => {
        server.off("error", reject);
        resolve();
      });
    });

  await listen();

  let fork: ForkHandle | undefined;
  if (config.fork.enabled) {
    fork = await startFork(config.fork);
  }

  return {
    server,
    port: config.port,
    logger,
    fork,
    close: () =>
      new Promise<void>((resolve, reject) => {
        fork?.stop();
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}

// -----------------------------------------------------------------------------
// CLI entrypoint
// -----------------------------------------------------------------------------

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const configFlagIdx = args.findIndex((a) => a === "--config" || a.startsWith("--config="));
  let configPath: string | undefined;
  if (configFlagIdx >= 0) {
    const arg = args[configFlagIdx];
    if (arg.includes("=")) configPath = arg.split("=")[1];
    else configPath = args[configFlagIdx + 1];
  }

  const config = await loadConfig(configPath);
  if (!config.enabled) {
    console.log("[devProxy] disabled by config");
    return;
  }

  const proxy = await createDevProxy({ config });
  console.log(`[devProxy] listening on http://127.0.0.1:${proxy.port} -> ${config.target}`);
  if (proxy.fork) console.log(`[devProxy] fork (${proxy.fork.tool}) at ${proxy.fork.url}`);

  const shutdown = async () => {
    await proxy.close();
    process.exit(0);
  };
  process.on("SIGINT", () => void shutdown());
  process.on("SIGTERM", () => void shutdown());
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
