/**
 * Client-side service worker registration (issue #688).
 *
 * Registers `/sw.js` on the first visit in production and reports when a new
 * version is waiting so the UI can show an update prompt. The worker only
 * takes over after {@link applyServiceWorkerUpdate} is called.
 */

export interface ServiceWorkerCallbacks {
  /** A new service worker version is installed and waiting. */
  onUpdateAvailable?: (registration: ServiceWorkerRegistration) => void;
  /** Failed mutations were queued (offline) or replayed (back online). */
  onMutationQueued?: () => void;
  onMutationsReplayed?: (count: number) => void;
  /** A signing/mutation request was blocked while offline (never submitted). */
  onOfflineActionBlocked?: (message: string) => void;
}

export function isServiceWorkerSupported(): boolean {
  return typeof window !== "undefined" && "serviceWorker" in navigator;
}

export async function registerServiceWorker(
  callbacks: ServiceWorkerCallbacks = {},
): Promise<ServiceWorkerRegistration | null> {
  if (!isServiceWorkerSupported()) return null;
  if (process.env.NODE_ENV !== "production") return null;

  const registration = await navigator.serviceWorker.register("/sw.js");

  // A worker may already be waiting from a previous visit.
  if (registration.waiting && navigator.serviceWorker.controller) {
    callbacks.onUpdateAvailable?.(registration);
  }

  registration.addEventListener("updatefound", () => {
    const installing = registration.installing;
    if (!installing) return;
    installing.addEventListener("statechange", () => {
      if (installing.state === "installed" && navigator.serviceWorker.controller) {
        callbacks.onUpdateAvailable?.(registration);
      }
    });
  });

  navigator.serviceWorker.addEventListener("message", (event: MessageEvent) => {
    const data = event.data as { type?: string; count?: number; message?: string } | undefined;
    if (data?.type === "MUTATION_QUEUED") callbacks.onMutationQueued?.();
    if (data?.type === "MUTATIONS_REPLAYED") callbacks.onMutationsReplayed?.(data.count ?? 0);
    if (data?.type === "OFFLINE_ACTION_BLOCKED") {
      callbacks.onOfflineActionBlocked?.(
        data.message ?? "You are offline. Reconnect and try again — this action was not submitted.",
      );
    }
  });

  return registration;
}

/** Activate the waiting worker and reload once it has taken control. */
export function applyServiceWorkerUpdate(registration: ServiceWorkerRegistration): void {
  const waiting = registration.waiting;
  if (!waiting) return;
  navigator.serviceWorker.addEventListener("controllerchange", () => window.location.reload(), {
    once: true,
  });
  waiting.postMessage({ type: "SKIP_WAITING" });
}
