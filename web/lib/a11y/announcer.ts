/**
 * Screen-reader announcement verbosity — issue #697.
 *
 * Announcing every state change is worse than announcing nothing: a balance
 * that re-renders on every poll, or a status that flips between three
 * equivalent phrasings, trains users to skip live regions entirely. The
 * verbosity level lets a person choose how much they hear, and the formatter
 * enforces that choice consistently for every caller.
 *
 * The preference is stored under `perigee:a11y:announcer-verbosity` and read
 * back on mount, so it survives reloads and applies to the first render
 * rather than only after the user changes it.
 *
 * @see components/AnnouncerProvider for the React binding.
 */

export type Verbosity = "terse" | "normal" | "verbose";

export const VERBOSITY_LEVELS: readonly Verbosity[] = [
  "terse",
  "normal",
  "verbose",
] as const;

export const VERBOSITY_STORAGE_KEY = "perigee:a11y:announcer-verbosity";

export const DEFAULT_VERBOSITY: Verbosity = "normal";

/** Coerce anything (localStorage, a URL param) into a known level. */
export function normalizeVerbosity(value: unknown): Verbosity {
  return VERBOSITY_LEVELS.includes(value as Verbosity)
    ? (value as Verbosity)
    : DEFAULT_VERBOSITY;
}

export interface FormatOptions {
  /** What to announce when the message is empty after trimming. */
  empty?: string;
  /** Appended in verbose mode to say where the update came from. */
  source?: string;
  /** Terse mode keeps at most this many characters (word boundary). */
  terseLimit?: number;
}

const TENSE_LIMIT = 80;

/**
 * Applies a verbosity level to a message.
 *
 * - `terse`   — first clause only, capped, punctuation dropped. Announcing
 *               "Balance updated" is the point; the numbers are on screen.
 * - `normal`  — the message as written (the default used everywhere before).
 * - `verbose` — message plus its source, so a user can tell which subsystem
 *               spoke.
 */
export function formatAnnouncement(
  message: string,
  verbosity: Verbosity = DEFAULT_VERBOSITY,
  options: FormatOptions = {},
): string {
  const text = (message ?? "").trim();

  if (!text) {
    return options.empty ?? "";
  }

  if (verbosity === "verbose" && options.source) {
    return `${text} (${options.source})`;
  }

  if (verbosity !== "terse") {
    return text;
  }

  const limit = options.terseLimit ?? TENSE_LIMIT;
  if (text.length <= limit) {
    return text.replace(/[.。!！]+$/u, "");
  }

  const clipped = text.slice(0, limit);
  const lastSpace = clipped.lastIndexOf(" ");
  const head = lastSpace > limit * 0.5 ? clipped.slice(0, lastSpace) : clipped;

  return `${head.replace(/[.。!！,，]+$/u, "")}…`;
}

/**
 * Successive identical announcements are ignored.
 *
 * Live regions only speak when their content *changes*, so re-announcing the
 * same string is a no-op at best and, when a counter is appended to force the
 * change, a source of noise at worst. Deduplicating here keeps the call sites
 * simple.
 */
export function shouldAnnounce(
  previous: string | null | undefined,
  next: string,
): boolean {
  const value = (next ?? "").trim();
  if (!value) return false;
  return value !== (previous ?? "").trim();
}

/** Reads the persisted preference, tolerating private-mode failures. */
export function readStoredVerbosity(
  storage: Pick<Storage, "getItem"> | null | undefined = typeof localStorage === "undefined"
    ? null
    : localStorage,
): Verbosity {
  try {
    return normalizeVerbosity(storage?.getItem(VERBOSITY_STORAGE_KEY));
  } catch {
    return DEFAULT_VERBOSITY;
  }
}

/** Persists the preference; failures (quota, private mode) are non-fatal. */
export function storeVerbosity(
  verbosity: Verbosity,
  storage: Pick<Storage, "setItem"> | null | undefined = typeof localStorage === "undefined"
    ? null
    : localStorage,
): void {
  try {
    storage?.setItem(VERBOSITY_STORAGE_KEY, normalizeVerbosity(verbosity));
  } catch {
    // Ignore: the preference is a convenience, not a requirement.
  }
}
