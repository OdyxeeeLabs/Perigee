/**
 * Multi-provider analytics drivers for Plausible, Umami, PostHog, and Internal self-hosted collector.
 *
 * Implements privacy-first, cookie-less dispatch with automatic batching
 * via modern browser APIs (fetchLater / sendBeacon / keepalive).
 */

import { TelemetryEvent } from "./telemetrySchema";

export type AnalyticsProviderType = "plausible" | "umami" | "posthog" | "internal" | "none";

export interface AnalyticsDriver {
  readonly name: AnalyticsProviderType;
  init(): void;
  track(event: TelemetryEvent): void;
  pageView(path: string, referrer?: string): void;
  destroy(): void;
}

function isBrowser(): boolean {
  return typeof window !== "undefined" && typeof document !== "undefined";
}

// ---------------------------------------------------------------------------
// 1. Plausible Driver
// ---------------------------------------------------------------------------
export class PlausibleDriver implements AnalyticsDriver {
  readonly name = "plausible";
  private domain: string;
  private host: string;

  constructor(domain?: string, host?: string) {
    this.domain = domain || (isBrowser() ? window.location.hostname : "perigee.app");
    this.host = host || process.env.NEXT_PUBLIC_PLAUSIBLE_HOST || "https://plausible.io";
  }

  init(): void {
    if (!isBrowser()) return;
    if (document.getElementById("plausible-telemetry-script")) return;

    const script = document.createElement("script");
    script.id = "plausible-telemetry-script";
    script.defer = true;
    script.dataset.domain = this.domain;
    script.src = `${this.host.replace(/\/$/, "")}/js/script.js`;
    document.head.appendChild(script);
  }

  track(event: TelemetryEvent): void {
    if (!isBrowser()) return;
    const win = window as unknown as { plausible?: (name: string, opts?: { props?: Record<string, unknown> }) => void };
    if (typeof win.plausible === "function") {
      win.plausible(event.name, { props: event.properties });
    }
  }

  pageView(path: string): void {
    this.track({ name: "pageview", properties: { path } });
  }

  destroy(): void {
    if (!isBrowser()) return;
    const el = document.getElementById("plausible-telemetry-script");
    if (el) el.remove();
  }
}

// ---------------------------------------------------------------------------
// 2. Umami Driver
// ---------------------------------------------------------------------------
export class UmamiDriver implements AnalyticsDriver {
  readonly name = "umami";
  private websiteId: string;
  private host: string;

  constructor(websiteId?: string, host?: string) {
    this.websiteId = websiteId || process.env.NEXT_PUBLIC_UMAMI_WEBSITE_ID || "";
    this.host = host || process.env.NEXT_PUBLIC_UMAMI_HOST || "https://analytics.umami.is";
  }

  init(): void {
    if (!isBrowser() || !this.websiteId) return;
    if (document.getElementById("umami-telemetry-script")) return;

    const script = document.createElement("script");
    script.id = "umami-telemetry-script";
    script.defer = true;
    script.dataset.websiteId = this.websiteId;
    script.src = `${this.host.replace(/\/$/, "")}/script.js`;
    document.head.appendChild(script);
  }

  track(event: TelemetryEvent): void {
    if (!isBrowser()) return;
    const win = window as unknown as { umami?: { track: (name: string, data?: Record<string, unknown>) => void } };
    if (typeof win.umami?.track === "function") {
      win.umami.track(event.name, event.properties);
    }
  }

  pageView(path: string, referrer?: string): void {
    if (!isBrowser()) return;
    const win = window as unknown as { umami?: { track: (data: Record<string, unknown>) => void } };
    if (typeof win.umami?.track === "function") {
      win.umami.track({ url: path, referrer });
    }
  }

  destroy(): void {
    if (!isBrowser()) return;
    const el = document.getElementById("umami-telemetry-script");
    if (el) el.remove();
  }
}

// ---------------------------------------------------------------------------
// 3. PostHog Driver (Self-hosted or Cloud)
// ---------------------------------------------------------------------------
export class PostHogDriver implements AnalyticsDriver {
  readonly name = "posthog";
  private apiKey: string;
  private host: string;

  constructor(apiKey?: string, host?: string) {
    this.apiKey = apiKey || process.env.NEXT_PUBLIC_POSTHOG_KEY || "";
    this.host = host || process.env.NEXT_PUBLIC_POSTHOG_HOST || "https://us.i.posthog.com";
  }

  init(): void {
    if (!isBrowser() || !this.apiKey) return;
    if (document.getElementById("posthog-telemetry-script")) return;

    // Load PostHog snippet dynamically with IP and cookie tracking disabled
    const script = document.createElement("script");
    script.id = "posthog-telemetry-script";
    script.defer = true;
    script.src = `${this.host.replace(/\/$/, "")}/static/array.js`;
    script.onload = () => {
      const win = window as unknown as { posthog?: { init: (key: string, opts: Record<string, unknown>) => void } };
      if (typeof win.posthog?.init === "function") {
        win.posthog.init(this.apiKey, {
          api_host: this.host,
          autocapture: false,
          capture_pageview: false,
          disable_session_recording: true,
          ip: false, // Privacy first: do not record IP
          persistence: "memory", // No tracking cookies
        });
      }
    };
    document.head.appendChild(script);
  }

  track(event: TelemetryEvent): void {
    if (!isBrowser()) return;
    const win = window as unknown as { posthog?: { capture: (name: string, props?: Record<string, unknown>) => void } };
    if (typeof win.posthog?.capture === "function") {
      win.posthog.capture(event.name, event.properties);
    }
  }

  pageView(path: string): void {
    this.track({ name: "$pageview", properties: { $current_url: path } });
  }

  destroy(): void {
    if (!isBrowser()) return;
    const el = document.getElementById("posthog-telemetry-script");
    if (el) el.remove();
  }
}

// ---------------------------------------------------------------------------
// 4. Internal Self-Hosted Collector Driver (Modern Web Guidance fetchLater / Batching)
// ---------------------------------------------------------------------------
const INTERNAL_ENDPOINT = "/api/telemetry";
const BATCH_WINDOW_MS = 3000;
const MAX_QUEUE_SIZE = 50;

export class InternalDriver implements AnalyticsDriver {
  readonly name = "internal";
  private eventQueue: TelemetryEvent[] = [];
  private batchTimer: ReturnType<typeof setTimeout> | null = null;
  private endpoint: string;

  constructor(endpoint?: string) {
    this.endpoint = endpoint || INTERNAL_ENDPOINT;
  }

  init(): void {
    if (!isBrowser()) return;
    // Flush on page unload or visibilitychange to ensure no lost events
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", this.handleVisibilityChange);
      window.addEventListener("pagehide", this.handlePageHide);
    }
  }

  track(event: TelemetryEvent): void {
    this.eventQueue.push(event);

    if (this.eventQueue.length >= MAX_QUEUE_SIZE) {
      this.flush();
      return;
    }

    if (!this.batchTimer) {
      this.batchTimer = setTimeout(() => this.flush(), BATCH_WINDOW_MS);
    }
  }

  pageView(path: string, referrer?: string): void {
    this.track({
      name: "page_view",
      category: "pageview",
      properties: { path, referrer: referrer || "" },
    });
  }

  flush(): void {
    if (this.batchTimer) {
      clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }

    if (this.eventQueue.length === 0 || !isBrowser()) return;

    const payload = JSON.stringify({ events: [...this.eventQueue] });
    this.eventQueue = [];

    // Use fetchLater if available (modern browser standard), otherwise sendBeacon or fetch keepalive
    type FetchLaterFn = (url: string, init?: { method?: string; headers?: Record<string, string>; body?: string; activateAfter?: number }) => void;
    const globalWithFetchLater = globalThis as unknown as { fetchLater?: FetchLaterFn };

    if (typeof globalWithFetchLater.fetchLater === "function") {
      try {
        globalWithFetchLater.fetchLater(this.endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: payload,
          activateAfter: 0,
        });
        return;
      } catch {
        // Fall back below
      }
    }

    if (typeof navigator.sendBeacon === "function") {
      const blob = new Blob([payload], { type: "application/json" });
      const sent = navigator.sendBeacon(this.endpoint, blob);
      if (sent) return;
    }

    // Keepalive fetch fallback
    void fetch(this.endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: payload,
      keepalive: true,
    }).catch(() => undefined);
  }

  private handleVisibilityChange = (): void => {
    if (document.visibilityState === "hidden") {
      this.flush();
    }
  };

  private handlePageHide = (): void => {
    this.flush();
  };

  destroy(): void {
    this.flush();
    if (isBrowser()) {
      document.removeEventListener("visibilitychange", this.handleVisibilityChange);
      window.removeEventListener("pagehide", this.handlePageHide);
    }
  }
}

// ---------------------------------------------------------------------------
// Provider Factory & Coordinator
// ---------------------------------------------------------------------------
export function createAnalyticsDriver(provider?: AnalyticsProviderType): AnalyticsDriver {
  const chosen = provider || (process.env.NEXT_PUBLIC_ANALYTICS_PROVIDER as AnalyticsProviderType) || "plausible";

  switch (chosen) {
    case "umami":
      return new UmamiDriver(process.env.NEXT_PUBLIC_UMAMI_WEBSITE_ID, process.env.NEXT_PUBLIC_UMAMI_HOST);
    case "posthog":
      return new PostHogDriver(process.env.NEXT_PUBLIC_POSTHOG_KEY, process.env.NEXT_PUBLIC_POSTHOG_HOST);
    case "internal":
      return new InternalDriver();
    case "none":
      return {
        name: "none",
        init() {},
        track() {},
        pageView() {},
        destroy() {},
      };
    case "plausible":
    default:
      return new PlausibleDriver(process.env.NEXT_PUBLIC_PLAUSIBLE_DOMAIN, process.env.NEXT_PUBLIC_PLAUSIBLE_HOST);
  }
}
