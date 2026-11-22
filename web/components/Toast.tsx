"use client";

/**
 * Toast viewport + item — issue #681 (presentation layer).
 *
 * `ToastViewport` stacks active toasts (newest first) in a fixed, high
 * z-index region. `ToastItem` owns its own lifecycle visuals:
 *   - auto-dismiss countdown, paused while the pointer is over it or it has
 *     keyboard focus (so a lingering cursor never eats a message),
 *   - swipe / drag to dismiss (handy on touch),
 *   - motion that fully respects `prefers-reduced-motion`.
 *
 * Announcements for assistive tech are handled centrally by `ToastProvider`
 * (single live region), so these elements intentionally avoid `aria-live`.
 *
 * Kept OUT of the docs registry on purpose — see the note in `ToastContext`.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
  AnimatePresence,
  motion,
  useReducedMotion,
  type PanInfo,
} from "framer-motion";
import {
  AlertTriangle,
  CheckCircle2,
  Info,
  X,
  XCircle,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { cn } from "../lib/utils";
import type { ToastRecord, ToastVariant } from "../context/ToastContext";

/** Visual recipe per variant: icon + accent + text colour on the dark surface. */
const VARIANT_CONFIG: Record<
  ToastVariant,
  { Icon: typeof Info; accent: string; iconClass: string }
> = {
  success: { Icon: CheckCircle2, accent: "#4ade80", iconClass: "text-[#4ade80]" },
  error: { Icon: XCircle, accent: "#f87171", iconClass: "text-[#f87171]" },
  warning: { Icon: AlertTriangle, accent: "#fbbf24", iconClass: "text-[#fbbf24]" },
  info: { Icon: Info, accent: "#60a5fa", iconClass: "text-[#60a5fa]" },
};

/** Distance (px) a toast must be dragged before it is dismissed. */
const DISMISS_DRAG_THRESHOLD = 96;

interface ToastItemProps {
  toast: ToastRecord;
  onDismiss: (id: string) => void;
}

export function ToastItem({ toast, onDismiss }: ToastItemProps) {
  const t = useTranslations("toast");
  const shouldReduceMotion = useReducedMotion();
  const [paused, setPaused] = useState(false);

  const remainingRef = useRef(toast.duration);
  const startedAtRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const close = useCallback(() => onDismiss(toast.id), [onDismiss, toast.id]);

  // Auto-dismiss timer. Recreated whenever pause state changes; the cleanup
  // banks the elapsed time into `remainingRef` so hovering only *defers* the
  // dismissal rather than resetting it. A `duration <= 0` toast is sticky.
  useEffect(() => {
    if (toast.duration <= 0) return;

    if (paused) return;

    startedAtRef.current = Date.now();
    timerRef.current = setTimeout(close, remainingRef.current);

    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
        const elapsed = Date.now() - startedAtRef.current;
        remainingRef.current = Math.max(0, remainingRef.current - elapsed);
      }
    };
  }, [paused, toast.duration, close]);

  const handleDragEnd = useCallback(
    (_event: MouseEvent | TouchEvent | PointerEvent, info: PanInfo) => {
      const dragged =
        Math.abs(info.offset.x) > DISMISS_DRAG_THRESHOLD ||
        Math.abs(info.offset.y) > DISMISS_DRAG_THRESHOLD;
      const flicked =
        Math.abs(info.velocity.x) > 500 || Math.abs(info.velocity.y) > 500;
      if (dragged || flicked) close();
    },
    [close],
  );

  const { Icon, accent, iconClass } = VARIANT_CONFIG[toast.variant];

  const slideOffset = shouldReduceMotion ? 0 : 24;

  return (
    <motion.div
      layout
      role="listitem"
      initial={
        shouldReduceMotion ? { opacity: 0 } : { opacity: 0, y: -slideOffset, scale: 0.96 }
      }
      animate={shouldReduceMotion ? { opacity: 1 } : { opacity: 1, y: 0, scale: 1 }}
      exit={shouldReduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.9 }}
      transition={{ duration: shouldReduceMotion ? 0 : 0.2, ease: "easeOut" }}
      drag={!shouldReduceMotion}
      dragDirectionLock
      dragElastic={0.55}
      dragMomentum={false}
      onDragEnd={handleDragEnd}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={() => setPaused(false)}
      className={cn(
        "pointer-events-auto relative w-full max-w-sm overflow-hidden rounded-xl",
        "border border-[#2A3338] bg-[#161E22] shadow-2xl",
        "select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#33C5E0]",
      )}
      style={{ touchAction: "pan-y" }}
      data-testid="toast-item"
      data-variant={toast.variant}
    >
      {/* Accent rail */}
      <span
        aria-hidden
        className="absolute inset-y-0 left-0 w-1"
        style={{ backgroundColor: accent }}
      />

      <div className="flex items-start gap-3 p-4 pl-5">
        <Icon className={cn("mt-0.5 h-5 w-5 shrink-0", iconClass)} aria-hidden />

        <div className="min-w-0 flex-1">
          {toast.title ? (
            <p className="text-sm font-medium text-white">{toast.title}</p>
          ) : null}
          {toast.description ? (
            <p className="mt-0.5 break-words text-sm text-[#92A5A8]">{toast.description}</p>
          ) : null}

          {toast.action ? (
            <button
              type="button"
              onClick={() => {
                toast.action?.onClick();
                close();
              }}
              className="mt-2 text-sm font-medium underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#33C5E0] rounded"
              style={{ color: accent }}
            >
              {toast.action.label}
            </button>
          ) : null}
        </div>

        <button
          type="button"
          onClick={close}
          aria-label={t("dismiss")}
          className="-mr-1 -mt-1 shrink-0 rounded-md p-1 text-[#92A5A8] transition-colors hover:bg-white/5 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#33C5E0]"
        >
          <X className="h-4 w-4" aria-hidden />
        </button>
      </div>
    </motion.div>
  );
}

interface ToastViewportProps {
  toasts: ToastRecord[];
  onDismiss: (id: string) => void;
}

export function ToastViewport({ toasts, onDismiss }: ToastViewportProps) {
  const t = useTranslations("toast");

  return (
    <div
      role="list"
      aria-label={t("regionLabel")}
      className="pointer-events-none fixed inset-x-0 bottom-0 z-[500] flex flex-col-reverse items-center gap-3 p-4 sm:inset-x-auto sm:right-0 sm:top-0 sm:bottom-auto sm:items-end sm:p-6"
    >
      <AnimatePresence initial={false}>
        {toasts.map((toast) => (
          <ToastItem key={toast.id} toast={toast} onDismiss={onDismiss} />
        ))}
      </AnimatePresence>
    </div>
  );
}
