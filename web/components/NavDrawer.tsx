"use client";

import React, { useEffect, useRef, useCallback } from "react";
import { useTranslations } from "next-intl";

interface NavDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  children: React.ReactNode;
  title?: string;
}

/**
 * Accessible mobile navigation drawer.
 *
 * Implements WCAG 2.1 requirements for dialog widgets:
 *  - role="dialog" + aria-modal="true" + aria-labelledby
 *  - Focus trap: Tab/Shift+Tab cycle within the drawer
 *  - Returns focus to the trigger element on close
 *  - Escape key closes the drawer
 *  - Backdrop click closes the drawer
 *
 * Resolves WEB-58 (#191): mobile nav drawer not accessible.
 */
export function NavDrawer({ isOpen, onClose, children, title }: NavDrawerProps) {
  const t = useTranslations();
  const resolvedTitle = title ?? t("nav.title");
  const drawerRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const titleId = "nav-drawer-title";

  // Save the element that triggered the drawer so we can restore focus on close
  useEffect(() => {
    if (isOpen) {
      previousFocusRef.current = document.activeElement as HTMLElement;
    }
  }, [isOpen]);

  // Move focus into the drawer when it opens; restore when it closes
  useEffect(() => {
    if (!isOpen) {
      previousFocusRef.current?.focus();
      return;
    }

    const focusable = getFocusable(drawerRef.current);
    focusable[0]?.focus();
  }, [isOpen]);

  // Trap Tab / Shift+Tab and handle Escape
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }

      if (e.key !== "Tab") return;

      const focusable = getFocusable(drawerRef.current);
      if (focusable.length === 0) return;

      const first = focusable[0];
      const last = focusable[focusable.length - 1];

      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    },
    [onClose],
  );

  // Prevent body scroll while drawer is open
  useEffect(() => {
    if (isOpen) {
      document.body.style.overflow = "hidden";
    } else {
      document.body.style.overflow = "";
    }
    return () => {
      document.body.style.overflow = "";
    };
  }, [isOpen]);

  if (!isOpen) return null;

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-40 bg-black/60"
        aria-hidden="true"
        onClick={onClose}
      />

      {/* Drawer panel */}
      <div
        ref={drawerRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="fixed inset-y-0 left-0 z-50 w-72 max-w-full bg-slate-900 shadow-xl flex flex-col"
        onKeyDown={handleKeyDown}
      >
        {/* Header with close button */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-slate-700">
          <span id={titleId} className="text-slate-100 font-semibold">
            {resolvedTitle}
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("nav.close")}
            className="rounded p-1 text-slate-400 hover:text-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-500"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeLinecap="round"
              strokeLinejoin="round"
              className="h-5 w-5"
              aria-hidden="true"
            >
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>

        {/* Drawer content */}
        <nav aria-label={t("nav.mobile")} className="flex-1 overflow-y-auto px-4 py-4">
          {children}
        </nav>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex=\"-1\"])",
].join(",");

function getFocusable(container: HTMLElement | null): HTMLElement[] {
  if (!container) return [];
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}
