import * as React from "react";
import { cn } from "../../lib/utils";

/**
 * Surface container from the Perigee component library.
 *
 * `Card` is a *layout* primitive only: it owns the border, radius, and
 * background, and has no opinion about content. Compose it with the header /
 * content / footer parts below so every panel in the app has identical spacing.
 *
 * @example
 * // Standard composed card
 * <Card>
 *   <CardHeader>
 *     <CardTitle>Vault performance</CardTitle>
 *     <CardDescription>Since inception, net of fees.</CardDescription>
 *   </CardHeader>
 *   <CardContent>{children}</CardContent>
 *   <CardFooter>
 *     <Button variant="ghost">View details</Button>
 *   </CardFooter>
 * </Card>
 *
 * @composition
 * The card is a compound component: every part is a plain `<div>` (or heading /
 * paragraph for the title and description) that renders in order. Only `Card`
 * itself adds visual chrome, so you can skip any part you do not need:
 *
 * - `CardHeader` — flex column, `p-6`, tight vertical rhythm.
 * - `CardTitle` — renders an `<h3>`; keep one per card for a clean outline.
 * - `CardDescription` — muted paragraph that sits under the title.
 * - `CardContent` — body area, `p-6` with the top padding removed.
 * - `CardFooter` — horizontal row for actions, `p-6` with the top padding
 *   removed.
 *
 * @accessibility
 * - `Card` accepts focus styling (`focus-visible:ring-2`) but is not focusable
 *   by default: only add `tabIndex` when the card itself is interactive, and
 *   prefer putting a real `<button>` inside `CardFooter`.
 * - Parts are plain containers, so heading order comes from your page, not from
 *   the component — `CardTitle` renders `<h3>`; do not skip heading levels.
 * - Cards are `role="group"`-free by default. When a card forms a labelled
 *   region, pass `role="group"` and `aria-labelledby` pointing at the title.
 *
 * @since 0.1.0
 */
const Card = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "rounded-lg border bg-card text-card-foreground shadow-sm",
      "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
      className,
    )}
    {...props}
  />
));
Card.displayName = "Card";

/** Header row of a {@link Card} — title and description, `p-6`. */
const CardHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex flex-col space-y-1.5 p-6", className)}
    {...props}
  />
));
CardHeader.displayName = "CardHeader";

/** Card heading. Renders an `<h3>`; use one per card to keep the outline valid. */
const CardTitle = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLHeadingElement>
>(({ className, ...props }, ref) => (
  <h3
    ref={ref}
    className={cn(
      "text-2xl font-semibold leading-none tracking-tight",
      className,
    )}
    {...props}
  />
));
CardTitle.displayName = "CardTitle";

/** Muted supporting text rendered directly below the {@link CardTitle}. */
const CardDescription = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => (
  <p ref={ref} className={cn("text-sm text-slate-300", className)} {...props} />
));
CardDescription.displayName = "CardDescription";

/** Body area of a {@link Card}; `p-6` with the top padding removed. */
const CardContent = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn("p-6 pt-0", className)} {...props} />
));
CardContent.displayName = "CardContent";

/** Action row at the bottom of a {@link Card} (buttons, links, metadata). */
const CardFooter = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex items-center p-6 pt-0", className)}
    {...props}
  />
));
CardFooter.displayName = "CardFooter";

export {
  Card,
  CardHeader,
  CardFooter,
  CardTitle,
  CardDescription,
  CardContent,
};
