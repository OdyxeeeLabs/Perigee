import * as React from "react";
import { cn } from "../../lib/utils";

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /**
   * Visual treatment. `"default"` is the cyan brand action; `"destructive"` is
   * reserved for irreversible actions; `"link"` renders text-only styling.
   */
  variant?: "default" | "destructive" | "outline" | "secondary" | "ghost" | "link";
  /** Control height and padding. `"icon"` is a square control for a single glyph. */
  size?: "default" | "sm" | "lg" | "icon";
  /**
   * Accepted for Radix-style API parity but **not implemented** yet: the
   * component always renders a `<button>`. Kept optional so callers migrating
   * from shadcn-based snippets do not fail type checking.
   */
  asChild?: boolean;
}

const variantStyles: Record<string, string> = {
  default: "bg-cyan-500 text-white hover:bg-cyan-600",
  destructive: "bg-red-600 text-white hover:bg-red-700",
  outline: "border border-slate-700 bg-transparent hover:bg-slate-800 text-slate-200",
  secondary: "bg-slate-800 text-slate-200 hover:bg-slate-700",
  ghost: "hover:bg-slate-800 text-slate-200",
  link: "text-cyan-400 underline-offset-4 hover:underline",
};

const sizeStyles: Record<string, string> = {
  default: "h-10 px-4 py-2",
  sm: "h-9 rounded-md px-3 text-xs",
  lg: "h-11 rounded-md px-8",
  icon: "h-10 w-10",
};

/**
 * Primary action control for the Perigee component library.
 *
 * Renders a native `<button>` and forwards every unhandled prop to it, so
 * `type`, `disabled`, `aria-*`, and event handlers behave exactly as they would
 * on a plain HTML button. Use `variant` to express emphasis and `size` for
 * density; keep a single `default` button per view for a clear primary action.
 *
 * Replaces the legacy `components/common/Button` (`CommonButton`), whose
 * `primary` variant maps onto `variant="default"`. See the
 * [migration guide](./migrations.md) for the full mapping.
 *
 * @example
 * // Primary call to action
 * <Button onClick={confirmRotation}>Confirm rotation</Button>
 *
 * @example
 * // Lower-emphasis destructive action
 * <Button variant="destructive" size="sm">Remove vault</Button>
 *
 * @accessibility
 * - Renders a real `<button>` whose default `type` is `button`, so keyboard
 *   activation (Enter / Space) works without extra handlers.
 * - Icon-only controls (`size="icon"`) have no text content: always pass an
 *   accessible name (`aria-label` / `aria-labelledby`) and mark the glyph
 *   `aria-hidden`.
 * - Focus is visible via `focus-visible:ring-2`; the ring must not be removed.
 * - `disabled` uses the native attribute, so the control leaves the tab order
 *   and is announced as unavailable. For a loading state that must stay
 *   focusable, combine `aria-busy` with `components/ui/Spinner` instead.
 *
 * @since 0.1.0
 */
const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "default", asChild: _asChild = false, ...props }, ref) => {
    return (
      <button
        className={cn(
          "inline-flex items-center justify-center whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 disabled:pointer-events-none disabled:opacity-50",
          variantStyles[variant] || variantStyles.default,
          sizeStyles[size] || sizeStyles.default,
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Button.displayName = "Button";

export { Button };
