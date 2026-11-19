"use client";

import { useCallback, useEffect, useState } from "react";

/**
 * High-contrast toggle — issue #701.
 *
 * Windows High Contrast Mode is handled in CSS under `@media (forced-colors:
 * active)` and needs no opt-in. This toggle exists for the other case: a user
 * who wants maximum contrast because of low vision, glare or a washed-out
 * display, on hardware where the OS does not offer that mode.
 *
 * The preference is an attribute on `<html>` (`data-contrast="high"`), applied
 * in an effect for the same reason `LocaleSync` does it in an effect: the
 * server cannot know the preference, so rendering it during the first client
 * render would be a hydration mismatch. The consequence is a brief flash of
 * normal contrast; the alternative (a blocking inline script) is worse for
 * everyone, so the tradeoff is documented in docs/accessibility/.
 */

export const CONTRAST_STORAGE_KEY = "perigee:contrast";
export type ContrastPreference = "normal" | "high";

const readPreference = (): ContrastPreference => {
  try {
    return localStorage.getItem(CONTRAST_STORAGE_KEY) === "high" ? "high" : "normal";
  } catch {
    return "normal";
  }
};

const writePreference = (value: ContrastPreference): void => {
  try {
    localStorage.setItem(CONTRAST_STORAGE_KEY, value);
  } catch {
    // A preference that cannot be stored must not break the toggle.
  }
};

export const applyContrast = (value: ContrastPreference): void => {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (value === "high") {
    root.setAttribute("data-contrast", "high");
  } else {
    root.removeAttribute("data-contrast");
  }
};

export function useContrastPreference() {
  const [preference, setPreference] = useState<ContrastPreference>("normal");

  useEffect(() => {
    const stored = readPreference();
    setPreference(stored);
    applyContrast(stored);
  }, []);

  const setValue = useCallback((value: ContrastPreference) => {
    setPreference(value);
    applyContrast(value);
    writePreference(value);
  }, []);

  return { preference, setPreference: setValue } as const;
}

export function HighContrastToggle({
  preference,
  onChange,
}: {
  preference?: ContrastPreference;
  onChange?: (value: ContrastPreference) => void;
}) {
  const [local, setLocal] = useState<ContrastPreference>(preference ?? "normal");

  const active = preference ?? local;

  return (
    <div className="flex items-center gap-2">
      <input
        id="high-contrast-toggle"
        type="checkbox"
        checked={active === "high"}
        onChange={(event) => {
          const next: ContrastPreference = event.target.checked ? "high" : "normal";
          setLocal(next);
          onChange?.(next);
        }}
      />
      <label htmlFor="high-contrast-toggle" className="text-sm">
        High contrast
      </label>
    </div>
  );
}

export default HighContrastToggle;
