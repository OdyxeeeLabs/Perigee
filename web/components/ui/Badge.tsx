import React from "react";
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";

function cn(...inputs: Parameters<typeof clsx>) {
  return twMerge(clsx(inputs));
}

type BadgeVariant = "default" | "success" | "warning" | "danger" | "info";

interface BadgeProps {
  /** Semantic colour pairing. Every variant is contrast-audited (see below). */
  variant?: BadgeVariant;
  /** Short label text — keep it to one or two words so the pill stays compact. */
  children: React.ReactNode;
  /** Extra Tailwind classes merged after the variant classes. */
  className?: string;
}

const variantClasses: Record<BadgeVariant, string> = {
  // Pairings audited for ≥ 4.5:1 on their tinted dark surfaces (WEB-38 / #492).
  default: "bg-slate-700 text-slate-200", // slate-200 on slate-700 ≈ 8.5:1
  success: "bg-emerald-950 text-emerald-300", // emerald-300 on emerald-950 ≈ 7+:1
  warning: "bg-amber-950 text-amber-300", // amber-300 on amber-950 ≈ 8+:1
  danger: "bg-rose-950 text-rose-300", // rose-300 on rose-950 ≈ 6+:1
  info: "bg-sky-950 text-sky-300", // sky-300 on sky-950 ≈ 7+:1
};

/**
 * Standardized Badge / pill component.
 *
 * Resolves WEB-27 (#113): component library not standardized.
 * Use for status labels, resource budget indicators, and severity tags.
 *
 * @example
 * <Badge variant="success">Passed</Badge>
 * <Badge variant="danger">Over Budget</Badge>
 *
 * @accessibility
 * - Colour is never the only signal: the text label carries the meaning, and
 *   every variant pairing is audited for ≥ 4.5:1 contrast on its tinted
 *   background (WEB-38 / #492).
 * - The label is plain text inside a `<span>`, so it is read as part of the
 *   surrounding sentence — do not use it as the sole status announcement for
 *   dynamic updates; pair with `LiveStatus` / `LiveAlert`.
 *
 * @since 0.1.0
 */
export const Badge = React.memo(function Badge({
  variant = "default",
  children,
  className,
}: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium",
        variantClasses[variant],
        className,
      )}
    >
      {children}
    </span>
  );
});
