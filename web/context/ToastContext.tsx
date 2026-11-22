"use client";

/**
 * Global toast / notification system — issue #681.
 *
 * Provides an app-wide notification queue with four variants (success, error,
 * warning, info). Any component can reach it through `useToast()`:
 *
 * ```tsx
 * const toast = useToast();
 * toast.success({ title: "Vault created", action: { label: "View", onClick } });
 * toast.error({ title: "Transaction failed", description: err.message });
 * ```
 *
 * Design goals:
 * - **Queue management** — at most `MAX_VISIBLE` toasts on screen; the oldest
 *   is evicted when the cap is exceeded.
 * - **Auto-dismiss** — every toast carries a configurable `duration`
 *   (`0` keeps it until dismissed). Timing/pause live in `ToastItem`.
 * - **Sound** — optional, opt-in audio cue per variant (see `lib/toastSound`).
 * - **Accessibility** — a single always-mounted live region announces each
 *   new message; errors use an assertive region, everything else polite. This
 *   mirrors the `components/LiveRegion` guidance (keep the region mounted and
 *   change only its text so screen readers actually announce updates).
 *
 * This is a UI feature component, deliberately kept OUT of the docs
 * `scripts/docs/registry.mjs` so `docs:check` / `component-docs.test.cjs`
 * (which assert a fixed set of documented components) stay green.
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ToastViewport } from "../components/Toast";
import { LiveAlert, LiveStatus } from "../components/LiveRegion";
import { playToastSound } from "../lib/toastSound";

export type ToastVariant = "success" | "error" | "warning" | "info";

/** Optional action button rendered inside a toast (e.g. "Retry", "View Tx"). */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

/** Caller-facing options when pushing a toast. */
export interface ToastOptions {
  /** Headline. When omitted for known variants a generic label can be supplied by callers. */
  title?: string;
  description?: string;
  /** Auto-dismiss delay in ms. `0` makes the toast sticky. */
  duration?: number;
  action?: ToastAction;
  /** Stable id (auto-generated when omitted). */
  id?: string;
}

/** Normalised toast stored in the queue. */
export interface ToastRecord {
  id: string;
  variant: ToastVariant;
  title: string;
  description?: string;
  duration: number;
  action?: ToastAction;
}

/** Default auto-dismiss window for transient toasts. */
export const DEFAULT_TOAST_DURATION = 5000;
/** Errors linger a little longer so they can be read. */
export const DEFAULT_ERROR_DURATION = 7000;
/** Hard cap on simultaneously visible toasts. */
export const MAX_VISIBLE_TOASTS = 5;

const SOUND_STORAGE_KEY = "perigee_toast_sound";

/** The public surface returned by `useToast()`. */
export interface ToastApi {
  show: (variant: ToastVariant, options: ToastOptions) => string;
  success: (options: ToastOptions) => string;
  error: (options: ToastOptions) => string;
  warning: (options: ToastOptions) => string;
  info: (options: ToastOptions) => string;
  dismiss: (id: string) => void;
  clear: () => void;
  /** Whether audible cues are enabled. */
  soundEnabled: boolean;
  /** Toggle audible cues (persisted to localStorage). */
  setSoundEnabled: (enabled: boolean) => void;
}

function generateId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `toast-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

const NoopToastApi: ToastApi = {
  show: () => "",
  success: () => "",
  error: () => "",
  warning: () => "",
  info: () => "",
  dismiss: () => {},
  clear: () => {},
  soundEnabled: false,
  setSoundEnabled: () => {},
};

const ToastContext = createContext<ToastApi>(NoopToastApi);

interface Announcement {
  message: string;
  variant: ToastVariant;
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastRecord[]>([]);
  const [announcement, setAnnouncement] = useState<Announcement | null>(null);
  const [soundEnabled, setSoundEnabledState] = useState(false);
  const soundEnabledRef = useRef(false);

  // Restore the persisted sound preference once on mount (client only) so the
  // initial render matches the server (both start "off").
  useEffect(() => {
    try {
      const stored = localStorage.getItem(SOUND_STORAGE_KEY);
      if (stored === "1") {
        soundEnabledRef.current = true;
        setSoundEnabledState(true);
      }
    } catch {
      // localStorage can throw in private modes / sandboxed frames — ignore.
    }
  }, []);

  const setSoundEnabled = useCallback((enabled: boolean) => {
    soundEnabledRef.current = enabled;
    setSoundEnabledState(enabled);
    try {
      localStorage.setItem(SOUND_STORAGE_KEY, enabled ? "1" : "0");
    } catch {
      // Ignore persistence failures.
    }
  }, []);

  const dismiss = useCallback((id: string) => {
    setToasts((prev) => prev.filter((toast) => toast.id !== id));
  }, []);

  const clear = useCallback(() => setToasts([]), []);

  const show = useCallback((variant: ToastVariant, options: ToastOptions) => {
    const record: ToastRecord = {
      id: options.id ?? generateId(),
      variant,
      title: options.title ?? "",
      description: options.description,
      duration:
        options.duration ??
        (variant === "error" ? DEFAULT_ERROR_DURATION : DEFAULT_TOAST_DURATION),
      action: options.action,
    };

    setToasts((prev) => {
      const next = [record, ...prev];
      return next.length > MAX_VISIBLE_TOASTS ? next.slice(0, MAX_VISIBLE_TOASTS) : next;
    });

    // Announce to assistive tech: assertive for errors, polite otherwise.
    const message = [record.title, record.description].filter(Boolean).join(". ");
    setAnnouncement({ message, variant });

    if (soundEnabledRef.current) {
      playToastSound(variant);
    }

    return record.id;
  }, []);

  const success = useCallback((o: ToastOptions) => show("success", o), [show]);
  const error = useCallback((o: ToastOptions) => show("error", o), [show]);
  const warning = useCallback((o: ToastOptions) => show("warning", o), [show]);
  const info = useCallback((o: ToastOptions) => show("info", o), [show]);

  const api = useMemo<ToastApi>(
    () => ({
      show,
      success,
      error,
      warning,
      info,
      dismiss,
      clear,
      soundEnabled,
      setSoundEnabled,
    }),
    [show, success, error, warning, info, dismiss, clear, soundEnabled, setSoundEnabled],
  );

  const isError = announcement?.variant === "error";

  return (
    <ToastContext.Provider value={api}>
      {children}
      <ToastViewport toasts={toasts} onDismiss={dismiss} />
      {/*
        Always-mounted live regions. `aria-atomic` re-announces the full text
        whenever it changes; a non-error toast clears the alert and vice-versa
        so only one region speaks at a time.
      */}
      <LiveAlert message={isError && announcement ? announcement.message : ""} />
      <LiveStatus
        message={!isError && announcement ? announcement.message : ""}
        label="Notifications"
      />
    </ToastContext.Provider>
  );
}

/** Access the global toast API. Falls back to no-ops outside a provider. */
export function useToast(): ToastApi {
  return useContext(ToastContext);
}
