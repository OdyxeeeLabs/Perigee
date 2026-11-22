# Design Token System — Issue #713

A single source of truth for every visual constant in the Perigee web frontend.
Tokens are declared once in CSS and mirrored as TypeScript constants so every
layer of the stack — Tailwind utilities, CSS modules, and JS/TS code — consumes
the same values.

---

## Files

| File | Purpose |
|------|---------|
| `web/styles/variables.css` | Canonical CSS custom properties (`--token-name: value`) defined on `:root`. Imported first in `globals.css`. |
| `web/styles/tokens.ts` | Type-safe TypeScript constants that mirror `variables.css`. Use in canvas, SSR, tests, or animation code. |
| `web/tailwind.config.js` | Maps CSS custom properties to Tailwind utility classes (`bg-primary`, `rounded-card-md`, etc.). |

---

## Token Categories

### Color

Declared under `/* Brand Colors */` and semantic groups in `variables.css`.

| CSS variable | Tailwind class | TypeScript | Default |
|---|---|---|---|
| `--color-primary` | `bg-primary` / `text-primary` | `tokens.color.primary` | `#2563eb` |
| `--color-primary-hover` | `bg-primary-hover` | `tokens.color.primaryHover` | `#1d4ed8` |
| `--color-primary-on-dark` | `text-primary-onDark` | `tokens.color.primaryOnDark` | `#60a5fa` |
| `--color-primary-foreground` | `text-primary-foreground` | `tokens.color.primaryForeground` | `#ffffff` |
| `--color-secondary` | `bg-secondary` | `tokens.color.secondary` | `#475569` |
| `--color-secondary-hover` | `bg-secondary-hover` | `tokens.color.secondaryHover` | `#334155` |
| `--color-success` | `bg-success` | `tokens.color.success` | `#15803d` |
| `--color-success-on-dark` | `bg-success-onDark` | `tokens.color.successOnDark` | `#4ade80` |
| `--color-warning` | `bg-warning` | `tokens.color.warning` | `#b45309` |
| `--color-warning-on-dark` | `bg-warning-onDark` | `tokens.color.warningOnDark` | `#fbbf24` |
| `--color-danger` | `bg-danger` | `tokens.color.danger` | `#b91c1c` |
| `--color-danger-on-dark` | `bg-danger-onDark` | `tokens.color.dangerOnDark` | `#f87171` |
| `--color-background` | `bg-canvas` | `tokens.color.background` | `#ffffff` |
| `--color-surface` | `bg-surface` | `tokens.color.surface` | `#f8fafc` |
| `--color-border` | `border-line` | `tokens.color.border` | `#64748b` |
| `--color-border-subtle` | `border-line-subtle` | `tokens.color.borderSubtle` | `#94a3b8` |
| `--color-text` | `text-body` | `tokens.color.text` | `#0f172a` |
| `--color-text-muted` | `text-body-muted` | `tokens.color.textMuted` | `#475569` |
| `--color-card` | `bg-card` | `tokens.color.card` | `#0f172a` |
| `--color-card-foreground` | `text-card-foreground` | `tokens.color.cardForeground` | `#f1f5f9` |

All color pairings are WCAG AA audited (≥ 4.5:1 for normal text, ≥ 3:1 for UI
components). See comments in `variables.css` for per-token ratios.

---

### Typography — Font Sizes

| CSS variable | TypeScript | Value |
|---|---|---|
| `--font-size-xs` | `tokens.fontSize.xs` | `0.75rem` |
| `--font-size-sm` | `tokens.fontSize.sm` | `0.875rem` |
| `--font-size-md` | `tokens.fontSize.md` | `1rem` |
| `--font-size-lg` | `tokens.fontSize.lg` | `1.125rem` |
| `--font-size-xl` | `tokens.fontSize.xl` | `1.25rem` |

These map to Tailwind's default `text-xs` / `text-sm` / `text-base` / `text-lg`
/ `text-xl`. Prefer Tailwind utilities; the CSS variables exist for CSS modules.

---

### Typography — Font Weights

| CSS variable | TypeScript | Value |
|---|---|---|
| `--font-weight-medium` | `tokens.fontWeight.medium` | `500` |
| `--font-weight-semibold` | `tokens.fontWeight.semibold` | `600` |
| `--font-weight-bold` | `tokens.fontWeight.bold` | `700` |

---

### Typography — Line Heights

| CSS variable | TypeScript | Value |
|---|---|---|
| `--line-height-tight` | `tokens.lineHeight.tight` | `1.25` |
| `--line-height-snug` | `tokens.lineHeight.snug` | `1.375` |
| `--line-height-normal` | `tokens.lineHeight.normal` | `1.5` |
| `--line-height-relaxed` | `tokens.lineHeight.relaxed` | `1.625` |
| `--line-height-loose` | `tokens.lineHeight.loose` | `2` |

---

### Spacing

Used for layout-scale rhythm: padding between top-level sections and internal
component padding.

| CSS variable | Tailwind class | TypeScript | Value |
|---|---|---|---|
| `--space-section` | `p-section` / `gap-section` | `tokens.space.section` | `4rem` |
| `--space-component` | `p-component` / `gap-component` | `tokens.space.component` | `1.5rem` |

These are also wired into `tailwind.config.js` under `theme.extend.spacing`.

---

### Border Radius

Tailwind's built-in `sm`/`md`/`lg`/`xl`/`full` keys are preserved.
Token-based radii are namespaced under `card-*` and `pill` to avoid conflicts.

| CSS variable | Tailwind class | TypeScript | Value |
|---|---|---|---|
| `--radius-sm` | `rounded-card-sm` | `tokens.radius.sm` | `6px` |
| `--radius-md` | `rounded-card-md` | `tokens.radius.md` | `10px` |
| `--radius-lg` | `rounded-card-lg` | `tokens.radius.lg` | `14px` |
| `--radius-xl` | `rounded-card-xl` | `tokens.radius.xl` | `18px` |
| `--radius-full` | `rounded-pill` | `tokens.radius.full` | `9999px` |

---

### Shadows

Namespaced under `card-*` to avoid overriding Tailwind built-ins.

| CSS variable | Tailwind class | TypeScript | Value |
|---|---|---|---|
| `--shadow-sm` | `shadow-card-sm` | `tokens.shadow.sm` | `0 1px 2px rgba(0,0,0,.05)` |
| `--shadow-md` | `shadow-card-md` | `tokens.shadow.md` | `0 4px 12px rgba(0,0,0,.08)` |
| `--shadow-lg` | `shadow-card-lg` | `tokens.shadow.lg` | `0 10px 24px rgba(0,0,0,.12)` |

---

### Transitions

| CSS variable | TypeScript | Value |
|---|---|---|
| `--transition-fast` | `tokens.transition.fast` | `.15s ease` |
| `--transition-normal` | `tokens.transition.normal` | `.25s ease` |
| `--transition-slow` | `tokens.transition.slow` | `.35s ease` |

Used in `Button.module.css` and other CSS modules via `var(--transition-fast)`.

---

### Z-Index

Defines a named, ordered stacking context ladder so layers never collide.

| CSS variable | TypeScript | Value | Use case |
|---|---|---|---|
| `--z-sticky` | `tokens.zIndex.sticky` | `10` | Sticky table headers, pinned side rails |
| `--z-dropdown` | `tokens.zIndex.dropdown` | `100` | Dropdowns, tooltips, floating UI |
| `--z-navbar` | `tokens.zIndex.navbar` | `200` | Fixed navigation bars, banners |
| `--z-drawer` | `tokens.zIndex.drawer` | `300` | Off-canvas drawers, side panels |
| `--z-modal` | `tokens.zIndex.modal` | `400` | Modal dialogs (overlay + panel) |
| `--z-toast` | `tokens.zIndex.toast` | `500` | Toast / notification stack |
| `--z-top` | `tokens.zIndex.top` | `600` | Dev-tools, command palette overlay |

---

## Usage Patterns

### 1. Tailwind utility class (preferred for React JSX)

```tsx
// bg-primary resolves through CSS custom property → rebrand in one place
<button className="bg-primary text-primary-foreground hover:bg-primary-hover rounded-card-md px-component">
  Connect Wallet
</button>
```

### 2. CSS Module (preferred for reusable component stylesheets)

```css
/* Button.module.css */
.button {
  background: var(--color-primary);
  border-radius: var(--radius-md);
  transition: background-color var(--transition-fast);
  padding: 0.5rem var(--space-component);
}
```

### 3. TypeScript constant (canvas, SSR, animation, tests)

```ts
import { tokens } from "@/styles/tokens";

// Canvas 2D
ctx.fillStyle = tokens.color.primary;

// Framer Motion
<motion.div animate={{ zIndex: tokens.zIndex.modal }} />

// Unit test assertion
expect(computedStyle.color).toBe(tokens.color.text);
```

### 4. Inline `style` prop (last resort — prefer CSS classes)

```tsx
import { tokens } from "@/styles/tokens";

// Only use when a Tailwind class or CSS var is genuinely not possible
<div style={{ zIndex: tokens.zIndex.drawer }}>…</div>
```

---

## Adding a New Token

1. Add the CSS custom property to `variables.css` under the appropriate section:
   ```css
   --color-accent: #0ea5e9;
   ```
2. Add the matching constant to `tokens.ts`:
   ```ts
   export const color = {
     …
     accent: "#0ea5e9",
   } as const;
   ```
3. If it needs a Tailwind utility, extend `tailwind.config.js`:
   ```js
   colors: {
     accent: "var(--color-accent, #0ea5e9)",
   }
   ```
4. Update this document's table.

---

## Rebrand / Theme Override

To change the primary brand color across the entire app:

```css
/* variables.css or a theme override file */
:root {
  --color-primary: #7c3aed; /* new brand violet */
  --color-primary-hover: #6d28d9;
  --color-primary-on-dark: #a78bfa;
}
```

Because every Tailwind color utility, CSS module, and TypeScript constant that
reads these variables will update automatically, no component-level changes are
needed.

---

## Accessibility Notes

- All foreground/background color pairings target **WCAG AA** (≥ 4.5:1 for
  normal text, ≥ 3:1 for large text and UI components).
- On-dark variants (`*-on-dark`) are tuned for use on `slate-950` backgrounds.
- Contrast ratios are annotated inline in `variables.css`.
