"use client";

import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Command } from "cmdk";
import { useRouter } from "next/router";
import { useTranslations } from "next-intl";
import { Search, FileText, Settings2, Zap } from "lucide-react";

import {
  ACTION_COMMANDS,
  NAVIGATION_COMMANDS,
  SETTINGS_COMMANDS,
  buildVaultCommands,
  filterCommands,
  groupCommands,
  groupHeadingKey,
  type CommandEntry,
} from "../lib/commandRegistry";
import { trackTelemetryEvent } from "../lib/telemetry";
import { useWalletStore } from "../context/WalletContext";

/**
 * Global command palette (WEB-52 / #185).
 *
 * Power users can jump to any vault, settings page, or run quick actions
 * from the keyboard. Open with ⌘K / Ctrl+K or the `/` key; Escape or
 * backdrop click closes it. Focus is trapped by cmdk's built-in dialog
 * behaviour and the trigger returns focus on close (WCAG 2.1 dialog
 * pattern, mirroring `NavDrawer.tsx`).
 *
 * Vault entries are dynamic: every vault known to the registry is
 * addressable via `/vault/<id>`, and typing a name or id filters to it.
 */

const OpenIcon = Search;
const NavIcon = FileText;
const SettingsIcon = Settings2;
const ActionIcon = Zap;

const GROUP_ICON = {
  navigation: NavIcon,
  vaults: NavIcon,
  settings: SettingsIcon,
  actions: ActionIcon,
} as const;

export interface CommandPaletteProps {
  /** Controlled open state (used by tests and the trigger button). */
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Stable a11y ids for the dialog pattern. */
const TITLE_ID = "command-palette-title";
const TRIGGER_ID = "command-palette-trigger";

export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const t = useTranslations();
  const router = useRouter();
  const wallet = useWalletStore((s) => ({ address: s.address }));

  const [query, setQuery] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // Remember what had focus before opening so we can restore it on close.
  useEffect(() => {
    if (open) {
      restoreFocusRef.current =
        document.activeElement as HTMLElement | null;
      setQuery("");
    } else if (restoreFocusRef.current) {
      restoreFocusRef.current.focus?.();
      restoreFocusRef.current = null;
    }
  }, [open]);

  // ⌘K / Ctrl+K toggles the palette; `/` opens it (when not typing in an
  // input/textarea/contenteditable), Escape closes.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        onOpenChange(!open);
        return;
      }

      if (!open && e.key === "/" && !isTypingTarget(e.target)) {
        e.preventDefault();
        onOpenChange(true);
        return;
      }

      if (open && e.key === "Escape") {
        e.preventDefault();
        onOpenChange(false);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onOpenChange]);

  // ---------------------------------------------------------------------------
  // Command sources
  // ---------------------------------------------------------------------------

  /** Resolves the display label for an entry (translated or plain vault name). */
  const resolveLabel = useCallback(
    (entry: CommandEntry): string => {
      switch (entry.kind) {
        case "vault":
          return entry.label;
        case "action":
        case "navigation":
        case "settings":
          return t(entry.labelKey);
        default:
          return "";
      }
    },
    [t]
  );

  const staticEntries = useMemo<CommandEntry[]>(
    () => [...NAVIGATION_COMMANDS, ...SETTINGS_COMMANDS, ...ACTION_COMMANDS],
    []
  );

  const handleRunAction = useCallback(
    (actionId: string) => {
      if (actionId === "copy-wallet-address") {
        const address = wallet.address;
        if (address) {
          navigator.clipboard
            .writeText(address)
            .then(() => trackTelemetryEvent({ name: "palette_copy_address" }))
            .catch(() => {
              /* clipboard unavailable — non-fatal */
            });
        }
      }
    },
    [wallet.address]
  );

  // ---------------------------------------------------------------------------
  // Derived list
  // ---------------------------------------------------------------------------

  const filtered = useMemo(
    () => filterCommands(staticEntries, query, resolveLabel),
    [staticEntries, query, resolveLabel]
  );

  const grouped = useMemo(() => groupCommands(filtered), [filtered]);

  const handleSelect = useCallback(
    (entry: CommandEntry) => {
      onOpenChange(false);
      trackTelemetryEvent({ name: "palette_navigate", properties: { id: entry.id } });
      if (entry.kind === "action") {
        handleRunAction(entry.actionId);
        return;
      }
      // Navigation, vault and settings entries all carry a route.
      if ("route" in entry && entry.route) {
        router.push(entry.route);
      }
    },
    [handleRunAction, onOpenChange, router]
  );

  return (
    <>
      <button
        ref={triggerRef}
        id={TRIGGER_ID}
        type="button"
        onClick={() => onOpenChange(true)}
        aria-haspopup="dialog"
        aria-expanded={open}
        className="flex items-center gap-2 rounded-md border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-500 hover:text-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-500"
      >
        <OpenIcon className="h-4 w-4" aria-hidden="true" />
        <span className="hidden sm:inline">{t("palette.triggerLabel")}</span>
        <kbd className="rounded border border-slate-600 px-1 text-[10px] text-slate-400">
          ⌘K
        </kbd>
      </button>

      <Command.Dialog
        open={open}
        onOpenChange={onOpenChange}
        loop
        label={t("palette.title")}
        className="fixed inset-0 z-50"
        overlayClassName="fixed inset-0 z-50 bg-black/60"
        contentClassName="fixed left-1/2 top-24 z-50 w-full max-w-xl -translate-x-1/2 rounded-lg border border-slate-700 bg-slate-900 shadow-xl outline-none"
      >
        <h2 id={TITLE_ID} className="sr-only">
          {t("palette.title")}
        </h2>
        <div className="flex items-center gap-2 border-b border-slate-700 px-4">
          <Search className="h-4 w-4 text-slate-400" aria-hidden="true" />
          <Command.Input
            value={query}
            onValueChange={setQuery}
            placeholder={t("palette.triggerLabel")}
            className="w-full bg-transparent py-3 text-sm text-slate-100 placeholder:text-slate-500 focus:outline-none"
            aria-label={t("palette.openLabel")}
          />
          <kbd className="rounded border border-slate-600 px-1 text-[10px] text-slate-400">
            ESC
          </kbd>
        </div>

        <Command.List className="max-h-80 overflow-y-auto p-2">
          <Command.Empty className="px-3 py-6 text-center text-sm text-slate-400">
            {t("palette.empty")}
            {query && (
              <span className="mt-1 block text-xs text-slate-500">
                {t("palette.searchVaultsHint")}
              </span>
            )}
          </Command.Empty>

          {grouped.map(({ group, entries }) => {
            const Icon = GROUP_ICON[group];
            return (
              <Command.Group
                key={group}
                heading={t(groupHeadingKey(group))}
                className="px-1 py-1 text-slate-300"
              >
                {entries.map((entry) => (
                  <Command.Item
                    key={entry.id}
                    value={`${resolveLabel(entry)} ${entry.keywords.join(" ")}`}
                    onSelect={() => handleSelect(entry)}
                    className="flex cursor-pointer items-center gap-2 rounded-md px-3 py-2 text-sm data-[selected=true]:bg-slate-800 data-[selected=true]:text-white"
                  >
                    <Icon className="h-4 w-4 text-slate-500" aria-hidden="true" />
                    <span className="flex-1">{resolveLabel(entry)}</span>
                    {entry.kind === "vault" && (
                      <span className="text-xs text-slate-500">{entry.vaultId}</span>
                    )}
                  </Command.Item>
                ))}
              </Command.Group>
            );
          })}
        </Command.List>
      </Command.Dialog>
    </>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** True when the event target is a text-entry element (don't hijack `/`). */
function isTypingTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}
