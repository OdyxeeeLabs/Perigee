"use client";

import { useId } from "react";
import { useAnnouncer } from "@/components/AnnouncerProvider";
import { VERBOSITY_LEVELS, type Verbosity } from "@/lib/a11y/announcer";

const LABELS: Record<Verbosity, string> = {
  terse: "Brief",
  normal: "Normal",
  verbose: "Detailed",
};

const HINTS: Record<Verbosity, string> = {
  terse: "Announces only the first clause of each update.",
  normal: "Announces updates in full.",
  verbose: "Announces updates with the source that produced them.",
};

/**
 * Verbosity control for live-region announcements — issue #697.
 *
 * Announcing every state change is actively harmful for people who rely on
 * live regions, so the amount of speech is a user preference rather than a
 * fixed behaviour. The choice is persisted by the announcer and applied from
 * the first render, so this control only has to render and report.
 */
export function AnnouncementVerbosityControl() {
  const { verbosity, setVerbosity } = useAnnouncer();
  const selectId = useId();
  const hintId = `${selectId}-hint`;

  return (
    <div className="flex items-center gap-2">
      <label htmlFor={selectId} className="text-sm">
        Announcements
      </label>
      <select
        id={selectId}
        value={verbosity}
        aria-describedby={hintId}
        onChange={(event) => setVerbosity(event.target.value as Verbosity)}
        className="rounded border px-2 py-1 text-sm"
      >
        {VERBOSITY_LEVELS.map((level) => (
          <option key={level} value={level}>
            {LABELS[level]}
          </option>
        ))}
      </select>
      <span id={hintId} className="sr-only">
        {HINTS[verbosity]}
      </span>
    </div>
  );
}

export default AnnouncementVerbosityControl;
