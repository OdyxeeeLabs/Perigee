/**
 * WCAG 2.1 contrast maths (issue #696).
 *
 * The design tokens in `styles/variables.css` document their ratios in
 * comments; this module computes them so the claims stay honest. It is
 * dependency-free and used both by the token test and by the audit doc.
 *
 * @see https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** WCAG 2.1 minimums. */
export const MIN_TEXT_CONTRAST = 4.5;
export const MIN_LARGE_TEXT_CONTRAST = 3;
export const MIN_NON_TEXT_CONTRAST = 3;

/**
 * Parses `#rgb`, `#rrggbb` (with or without a leading `#`) into channels.
 * Returns null for anything else so a typo surfaces as "unknown" rather than
 * silently reading as black.
 */
export function parseHexColor(value: string): Rgb | null {
  const hex = value.trim().replace(/^#/, '');

  if (/^[0-9a-f]{3}$/i.test(hex)) {
    return {
      r: parseInt(hex[0] + hex[0], 16),
      g: parseInt(hex[1] + hex[1], 16),
      b: parseInt(hex[2] + hex[2], 16),
    };
  }

  if (/^[0-9a-f]{6}$/i.test(hex)) {
    return {
      r: parseInt(hex.slice(0, 2), 16),
      g: parseInt(hex.slice(2, 4), 16),
      b: parseInt(hex.slice(4, 6), 16),
    };
  }

  return null;
}

/** Relative luminance per WCAG 2.1. */
export function relativeLuminance({ r, g, b }: Rgb): number {
  const channel = (raw: number): number => {
    const c = raw / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };

  return (
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
  );
}

/**
 * Contrast ratio between two colours, 1–21. Returns null when either value
 * cannot be parsed.
 */
export function contrastRatio(foreground: string, background: string): number | null {
  const fg = parseHexColor(foreground);
  const bg = parseHexColor(background);

  if (!fg || !bg) {
    return null;
  }

  const lighter = Math.max(relativeLuminance(fg), relativeLuminance(bg));
  const darker = Math.min(relativeLuminance(fg), relativeLuminance(bg));

  return (lighter + 0.05) / (darker + 0.05);
}

/** Rounded to 2dp so the audit doc and the test agree. */
export function roundedRatio(ratio: number | null): number | null {
  return ratio === null ? null : Math.round(ratio * 100) / 100;
}

export type ContrastGrade = 'pass-text' | 'pass-large' | 'fail';

/** Grades a pair for the given minimum, for use in reports. */
export function grade(
  foreground: string,
  background: string,
  minimum: number = MIN_TEXT_CONTRAST,
): { ratio: number | null; grade: ContrastGrade; passes: boolean } {
  const ratio = contrastRatio(foreground, background);
  const passes = ratio !== null && ratio >= minimum;

  return {
    ratio: roundedRatio(ratio),
    grade: passes
      ? minimum <= MIN_LARGE_TEXT_CONTRAST
        ? 'pass-text'
        : 'pass-text'
      : 'fail',
    passes,
  };
}
