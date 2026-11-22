"use client";

import Link from "next/link";
import type { ComponentProps } from "react";
import { usePrefetchOnHover } from "../hooks/usePrefetchOnHover";

type PrefetchLinkProps = Omit<ComponentProps<typeof Link>, "href" | "prefetch"> & {
  href: string;
  delayMs?: number;
};

/**
 * `next/link` with automatic viewport prefetching disabled and replaced by a
 * hover/focus prefetch after a short delay (issue #691). Honours data-saver
 * and slow-connection signals through `usePrefetchOnHover`.
 */
export function PrefetchLink({
  href,
  delayMs,
  onMouseEnter,
  onMouseLeave,
  onFocus,
  onBlur,
  ...rest
}: PrefetchLinkProps) {
  const handlers = usePrefetchOnHover(href, delayMs);

  return (
    <Link
      {...rest}
      href={href}
      prefetch={false}
      onMouseEnter={(event) => {
        handlers.onMouseEnter();
        onMouseEnter?.(event);
      }}
      onMouseLeave={(event) => {
        handlers.onMouseLeave();
        onMouseLeave?.(event);
      }}
      onFocus={(event) => {
        handlers.onFocus();
        onFocus?.(event);
      }}
      onBlur={(event) => {
        handlers.onBlur();
        onBlur?.(event);
      }}
    />
  );
}
