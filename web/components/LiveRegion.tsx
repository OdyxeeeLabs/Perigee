"use client";

import React from "react";

/**
 * Shared live regions for screen-reader announcements (WEB-16 / issue #470).
 *
 * Dynamic UI state — analysis progress, transaction outcomes, upload results,
 * network changes — is often rendered as purely visual feedback (spinners,
 * colour changes, toasts). Screen readers never see it. Wrapping those updates
 * in an element with `aria-live` makes assistive technology announce them as
 * the content changes.
 *
 * # Polite vs assertive
 *
 * - [`LiveStatus`] renders `aria-live="polite"`: announcements queue behind
 *   whatever the user is doing. Right default for progress and completion
 *   updates ("analysis complete").
 * - [`LiveAlert`] renders `role="alert"` (implicitly `aria-live="assertive"`
 *   and `aria-atomic="true"`): announcements interrupt immediately. Right
 *   default for failures the user must act on ("analysis failed: …").
 *
 * # Always mounted
 *
 * A live region only announces *changes* to its content after assistive
 * technology has discovered it — a region that mounts together with its
 * message is frequently silent. Both components therefore render their
 * element even when `message` is empty; callers keep them mounted and simply
 * change the message text.
 *
 * # Visual impact: none
 *
 * The region is `sr-only` — announced but invisible — so it can sit next to
 * existing visual indicators without any layout change.
 */

/**
 * Polite live region for progress and completion updates.
 *
 * @example
 * <LiveStatus message={analysisComplete ? 'Analysis complete.' : ''} />
 *
 * @since 0.1.0
 */
export interface LiveStatusProps {
  /** The current status message. Rendered inside the region; empty = silent. */
  message: string;
  /**
   * Optional label naming the region ("Status updates") so screen-reader
   * users can find it when browsing landmarks.
   */
  label?: string;
  /** Test hook. */
  "data-testid"?: string;
}

export function LiveStatus({ message, label, "data-testid": testId }: LiveStatusProps) {
  return (
    <div
      aria-live="polite"
      aria-atomic="true"
      role="status"
      aria-label={label}
      data-testid={testId}
      className="sr-only"
    >
      {message}
    </div>
  );
}

/**
 * Assertive live region for failures the user must act on.
 *
 * @example
 * <LiveAlert message={error ? `Analysis failed: ${error}` : ''} />
 *
 * @accessibility
 * - Renders `role="alert"` with `aria-live="assertive"` and
 *   `aria-atomic="true"`, so the whole message is re-announced on every
 *   change. Reserve it for errors: assertive announcements interrupt the user.
 * - Keep the component mounted and change only `message`; a region that mounts
 *   together with its text is frequently not announced.
 *
 * @since 0.1.0
 */
export interface LiveAlertProps {
  /** The alert message. Rendered inside the region; empty = silent. */
  message: string;
  /** Test hook. */
  "data-testid"?: string;
}

export function LiveAlert({ message, "data-testid": testId }: LiveAlertProps) {
  return (
    <div
      role="alert"
      aria-live="assertive"
      aria-atomic="true"
      data-testid={testId}
      className="sr-only"
    >
      {message}
    </div>
  );
}
