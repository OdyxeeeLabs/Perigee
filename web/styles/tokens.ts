/**
 * Design token constants — issue #713
 *
 * Single source of truth for design tokens in TypeScript contexts where
 * CSS custom properties are not directly accessible (e.g. canvas 2D, WebGL,
 * server-side rendering, unit tests, or programmatic animation values).
 *
 * ALL values here MUST stay in sync with `web/styles/variables.css`.
 * If you change a value in `variables.css`, update the matching constant here.
 *
 * Usage:
 *   import { tokens } from "@/styles/tokens";
 *   ctx.fillStyle = tokens.color.primary;           // canvas
 *   const delay = parseFloat(tokens.transition.fast); // animation
 *
 * For React component styling, prefer CSS custom properties via Tailwind
 * utility classes (e.g. `bg-primary`, `text-body`) over importing tokens
 * directly — the CSS variables respond to theme overrides at runtime.
 */

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

export const color = {
  // Brand
  primary: "#2563eb",
  primaryHover: "#1d4ed8",
  primaryOnDark: "#60a5fa",
  primaryForeground: "#ffffff",

  // Secondary
  secondary: "#475569",
  secondaryHover: "#334155",

  // Semantic
  success: "#15803d",
  successOnDark: "#4ade80",
  warning: "#b45309",
  warningOnDark: "#fbbf24",
  danger: "#b91c1c",
  dangerOnDark: "#f87171",

  // Surfaces
  background: "#ffffff",
  surface: "#f8fafc",

  // Borders
  border: "#64748b",
  borderSubtle: "#94a3b8",

  // Text
  text: "#0f172a",
  textMuted: "#475569",

  // Card (shadcn-style)
  card: "#0f172a",
  cardForeground: "#f1f5f9",
} as const;

export type ColorToken = keyof typeof color;

// ---------------------------------------------------------------------------
// Typography — font sizes
// ---------------------------------------------------------------------------

export const fontSize = {
  xs: "0.75rem",
  sm: "0.875rem",
  md: "1rem",
  lg: "1.125rem",
  xl: "1.25rem",
} as const;

export type FontSizeToken = keyof typeof fontSize;

// ---------------------------------------------------------------------------
// Typography — font weights
// ---------------------------------------------------------------------------

export const fontWeight = {
  medium: 500,
  semibold: 600,
  bold: 700,
} as const;

export type FontWeightToken = keyof typeof fontWeight;

// ---------------------------------------------------------------------------
// Typography — line heights
// ---------------------------------------------------------------------------

export const lineHeight = {
  tight: 1.25,
  snug: 1.375,
  normal: 1.5,
  relaxed: 1.625,
  loose: 2,
} as const;

export type LineHeightToken = keyof typeof lineHeight;

// ---------------------------------------------------------------------------
// Spacing
// ---------------------------------------------------------------------------

export const space = {
  /** Major vertical rhythm between top-level page sections. */
  section: "4rem",
  /** Internal padding / gap within a component (card body, form group). */
  component: "1.5rem",
} as const;

export type SpaceToken = keyof typeof space;

// ---------------------------------------------------------------------------
// Border radius
// ---------------------------------------------------------------------------

export const radius = {
  sm: "6px",
  md: "10px",
  lg: "14px",
  xl: "18px",
  full: "9999px",
} as const;

export type RadiusToken = keyof typeof radius;

// ---------------------------------------------------------------------------
// Shadows
// ---------------------------------------------------------------------------

export const shadow = {
  sm: "0 1px 2px rgba(0,0,0,.05)",
  md: "0 4px 12px rgba(0,0,0,.08)",
  lg: "0 10px 24px rgba(0,0,0,.12)",
} as const;

export type ShadowToken = keyof typeof shadow;

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

export const transition = {
  fast: ".15s ease",
  normal: ".25s ease",
  slow: ".35s ease",
} as const;

export type TransitionToken = keyof typeof transition;

// ---------------------------------------------------------------------------
// Z-Index
// ---------------------------------------------------------------------------

export const zIndex = {
  /** Sticky table headers, pinned side rails. */
  sticky: 10,
  /** Dropdowns, tooltips, floating UI anchored to a trigger. */
  dropdown: 100,
  /** Fixed navigation bars, always-visible banners. */
  navbar: 200,
  /** Drawers / off-canvas side panels. */
  drawer: 300,
  /** Modal dialogs (overlay + panel). */
  modal: 400,
  /** Toast / notification stack rendered above modals. */
  toast: 500,
  /** Absolute top: dev-tools, command palette overlay. */
  top: 600,
} as const;

export type ZIndexToken = keyof typeof zIndex;

// ---------------------------------------------------------------------------
// Aggregate export
// ---------------------------------------------------------------------------

/**
 * Namespaced design token object.
 *
 * @example
 * import { tokens } from "@/styles/tokens";
 * const style = { color: tokens.color.primary, zIndex: tokens.zIndex.modal };
 */
export const tokens = {
  color,
  fontSize,
  fontWeight,
  lineHeight,
  space,
  radius,
  shadow,
  transition,
  zIndex,
} as const;

export type Tokens = typeof tokens;
