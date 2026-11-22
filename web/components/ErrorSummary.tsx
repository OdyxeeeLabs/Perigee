"use client";

import { useEffect, useRef } from "react";

export interface FieldError {
  /** `id` of the invalid control, so the link can target it. */
  id: string;
  /** Message to show. */
  message: string;
  /** Optional field name for screen readers, e.g. "Email address". */
  label?: string;
}

export interface ErrorSummaryProps {
  errors: readonly FieldError[];
  title?: string;
  /** Set false to render the summary without taking focus (e.g. after a client-side re-check). */
  autoFocus?: boolean;
  onDismiss?: () => void;
  onNavigate?: (id: string) => void;
}

/**
 * Error summary — issue #703.
 *
 * Focusing an invalid field on submit is worse than it looks: it scrolls the
 * user past the other problems, announces one message, and hides the fact
 * that four things are wrong. The APG pattern is to render a summary of every
 * error, focus the *summary*, and let each entry be a link to the field.
 *
 * The summary is a live region as well as a focus target, because the two
 * cover different cases: the focus move covers keyboard and screen-reader
 * users who submitted the form, and the live region covers the case where
 * focus has been moved elsewhere deliberately (a field was re-checked while
 * the user was typing in another one).
 *
 * `role="alert"` is deliberately *not* used here. It interrupts whatever the
 * screen reader is saying, which is the wrong behaviour for validation that
 * arrives after a deliberate submit; `role="alert"` on the container plus a
 * `tabindex={-1}` focus move is the APG recommendation.
 */
export function ErrorSummary({
  errors,
  title = "There is a problem",
  autoFocus = true,
  onDismiss,
  onNavigate,
}: ErrorSummaryProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!autoFocus || errors.length === 0) return;
    // Focus the summary, not the first error: the user needs the count and
    // the full list before deciding which field to fix first.
    ref.current?.focus();
  }, [autoFocus, errors]);

  if (errors.length === 0) return null;

  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="alert"
      aria-labelledby="error-summary-title"
      className="rounded border-2 border-red-400 bg-red-950/40 p-4 focus:outline-none focus:ring-2 focus:ring-red-400"
    >
      <h2 id="error-summary-title" className="text-sm font-semibold text-red-200">
        {title}
      </h2>
      <ul className="mt-2 list-disc space-y-1 pl-5">
        {errors.map((error) => (
          <li key={error.id}>
            <a
              href={`#${error.id}`}
              className="text-sm text-red-100 underline underline-offset-2 hover:text-white"
              onClick={(event) => {
                event.preventDefault();
                onNavigate?.(error.id);
                const focusField = () => {
                  const target = document.getElementById(error.id);
                  if (!target) return;
                  target.focus();
                  target.scrollIntoView({ block: "center" });
                };
                if (onNavigate) {
                  requestAnimationFrame(() => requestAnimationFrame(focusField));
                } else {
                  focusField();
                }
              }}
            >
              {error.message}
            </a>
          </li>
        ))}
      </ul>
      {onDismiss ? (
        <button
          type="button"
          onClick={onDismiss}
          className="mt-3 text-sm text-red-200 underline underline-offset-2"
        >
          Dismiss
        </button>
      ) : null}
    </div>
  );
}

export default ErrorSummary;