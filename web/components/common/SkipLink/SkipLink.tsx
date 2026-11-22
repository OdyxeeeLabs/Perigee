"use client";

import { useEffect, useRef } from "react";

/**
 * Skip-to-main-content link — WCAG 2.1 SC 2.4.1 (Bypass Blocks), issue
 * #696 / #698.
 *
 * The app renders a command launcher, two network banners, an error boundary
 * and the analytics layer above the page content. A keyboard user therefore
 * tabs through all of that on every route change. This link is the first
 * focusable element on the page, is visually hidden until focused, and moves
 * focus (not just the scroll position) to `<main id="main-content">`.
 *
 * The target is given `tabIndex={-1}` so it can receive programmatic focus;
 * see the `<main>` wrapper rendered in `pages/_app.tsx`.
 */
export function SkipLink({
  targetId = "main-content",
  children = "Skip to main content",
}: {
  targetId?: string;
  children?: React.ReactNode;
}) {
  const linkRef = useRef<HTMLAnchorElement | null>(null);

  useEffect(() => {
    // Clicking the link with a pointer does not always move focus in every
    // browser; do it explicitly so the next Tab continues from the main
    // content rather than from the top of the document.
    const onClick = () => {
      const target = document.getElementById(targetId);
      if (!target) return;
      target.focus({ preventScroll: false });
      target.scrollIntoView?.({ block: "start" });
    };

    const node = linkRef.current;
    node?.addEventListener("click", onClick);
    return () => node?.removeEventListener("click", onClick);
  }, [targetId]);

  return (
    <a ref={linkRef} href={`#${targetId}`} className="skip-link">
      {children}
    </a>
  );
}

export default SkipLink;
