import React from "react";
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";

function cn(...inputs: Parameters<typeof clsx>) {
  return twMerge(clsx(inputs));
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type SpinnerSize = "xs" | "sm" | "md" | "lg" | "xl";
export type SpinnerColor = "primary" | "white" | "muted" | "danger" | "success";

export interface SpinnerProps {
  /** Controls the outer diameter of the spinner ring. Defaults to "md". */
  size?: SpinnerSize;
  /**
   * Color token for the spinning arc. "primary" matches the Perigee brand
   * cyan (#33C5E0). Defaults to "primary".
   */
  color?: SpinnerColor;
  /**
   * Optional accessible label announced by screen readers.
   * When omitted the spinner is purely decorative (aria-hidden="true").
   */
  label?: string;
  /** Extra Tailwind classes forwarded to the root element. */
  className?: string;
}

// ---------------------------------------------------------------------------
// Style maps
// ---------------------------------------------------------------------------

const sizeClasses: Record<SpinnerSize, string> = {
  xs: "w-3 h-3 border-[1.5px]",
  sm: "w-4 h-4 border-2",
  md: "w-6 h-6 border-2",
  lg: "w-8 h-8 border-[3px]",
  xl: "w-12 h-12 border-4",
};

const colorClasses: Record<SpinnerColor, string> = {
  primary: "border-[#33C5E0]/20 border-t-[#33C5E0]",
  white: "border-white/20   border-t-white",
  muted: "border-slate-600  border-t-slate-300",
  danger: "border-rose-800   border-t-rose-400",
  success: "border-emerald-800 border-t-emerald-400",
};

const labelSizeClasses: Record<SpinnerSize, string> = {
  xs: "text-[10px]",
  sm: "text-xs",
  md: "text-sm",
  lg: "text-base",
  xl: "text-lg",
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/**
 * Standardised loading spinner for the Perigee component library.
 *
 * Resolves the ad-hoc loading indicator inconsistency noted across multiple
 * components. Use this instead of inline SVGs or Lucide's Loader2 so that
 * size, color, and motion behaviour are uniform app-wide.
 *
 * @example
 * // Decorative (no announcement)
 * <Spinner size="sm" color="primary" />
 *
 * // Accessible (announces to screen readers)
 * <Spinner size="md" label="Loading vault data…" />
 * *   // Custom color with label below the ring
 *   <Spinner size="lg" color="white" label="Connecting…" />
 *
 * @accessibility
 * - Without `label` the ring is `aria-hidden="true"`, which is correct inside a
 *   button that already has a name ("Saving…").
 * - With `label` the ring becomes `role="status"`, which announces politely;
 *   for failures that must interrupt, use `LiveAlert` instead.
 * - Under `prefers-reduced-motion: reduce` the global stylesheet keeps the
 *   ring turning at 3s rather than stopping it: a stopped ring is
 *   indistinguishable from a broken UI. The rule is therefore not "disable
 *   the spinner" but "never let the spinner be the only signal" — always pass
 *   `label`, or pair it with adjacent text. See
 *   `docs/accessibility/reduced-motion.md` and `lib/a11y/motion.ts`
 *   (`loaderCue`), which encodes the same contract.
 *
 * @since 0.1.0
 */
export const Spinner = React.memo(function Spinner({
  size = "md",
  color = "primary",
  label,
  className,
}: SpinnerProps) {
  const ring = (
    <span
      role="status"
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cn(
        "inline-block rounded-full animate-spin",
        sizeClasses[size],
        colorClasses[color],
        className,
      )}
    />
  );

  if (!label) {
    return ring;
  }

  return (
    <span className="inline-flex flex-col items-center gap-1.5">
      {ring}
      <span
        className={cn(
          "text-current opacity-70 font-medium",
          labelSizeClasses[size],
        )}
      >
        {label}
      </span>
    </span>
  );
});
