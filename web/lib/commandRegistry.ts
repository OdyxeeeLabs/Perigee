/**
 * Command palette registry (WEB-52 / #185).
 *
 * Single source of truth for every entry the global command palette
 * (`components/CommandPalette.tsx`) offers: static destinations
 * (navigation + settings/admin), dynamic vault routes, and lightweight
 * client-side actions. Keeping the data separate from the UI makes the
 * palette testable and lets white-label deployments override labels via
 * next-intl messages without touching this file.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type CommandGroup = "navigation" | "vaults" | "settings" | "actions";

export interface NavigationCommand {
  kind: "navigation";
  id: string;
  route: string;
  /** next-intl message key for the label (e.g. `palette.goHome`). */
  labelKey: string;
  keywords: string[];
}

export interface VaultCommand {
  kind: "vault";
  id: string;
  route: string;
  /** Human-readable vault name from the registry (plain string, not a key). */
  label: string;
  vaultId: string;
  keywords: string[];
}

export interface SettingsCommand {
  kind: "settings";
  id: string;
  route: string;
  labelKey: string;
  keywords: string[];
}

export interface ActionCommand {
  kind: "action";
  id: string;
  /** `actionId` is passed back to the palette's `onRunAction` callback. */
  actionId: "copy-wallet-address";
  labelKey: string;
  keywords: string[];
}

export type CommandEntry =
  | NavigationCommand
  | VaultCommand
  | SettingsCommand
  | ActionCommand;

// ---------------------------------------------------------------------------
// Static entries — destinations a power user reaches most often
// ---------------------------------------------------------------------------

export const NAVIGATION_COMMANDS: NavigationCommand[] = [
  {
    kind: "navigation",
    id: "nav-home",
    route: "/",
    labelKey: "palette.goHome",
    keywords: ["home", "analyzer", "contract", "dashboard", "start"],
  },
  {
    kind: "navigation",
    id: "nav-vaults",
    route: "/vaults",
    labelKey: "palette.goVaults",
    keywords: ["vaults", "portfolio", "funds", "positions", "list"],
  },
];

export const SETTINGS_COMMANDS: SettingsCommand[] = [
  {
    kind: "settings",
    id: "settings-admin-managers",
    route: "/admin/managers",
    labelKey: "palette.goAdminManagers",
    keywords: [
      "admin",
      "managers",
      "kyc",
      "approve",
      "whitelist",
      "settings",
      "administration",
    ],
  },
  {
    kind: "settings",
    id: "settings-manager-onboarding",
    route: "/managers/onboarding",
    labelKey: "palette.goManagerOnboarding",
    keywords: [
      "onboarding",
      "manager",
      "register",
      "kyc",
      "signup",
      "sign up",
      "settings",
    ],
  },
];

/** Groups shown in the palette, in display order. */
export const COMMAND_GROUP_ORDER: CommandGroup[] = [
  "navigation",
  "vaults",
  "settings",
  "actions",
];

/**
 * Maps a palette group to its next-intl heading key
 * (e.g. `palette.groupNavigation`).
 */
export function groupHeadingKey(group: CommandGroup): string {
  return `palette.group${group.charAt(0).toUpperCase()}${group.slice(1)}`;
}

// ---------------------------------------------------------------------------
// Dynamic vault entries
// ---------------------------------------------------------------------------

/**
 * Minimal vault shape the registry needs. Structural on purpose so
 * `Vault`, `VaultRecord`, or plain API stubs can be passed directly.
 */
export interface VaultRegistryItem {
  id: string;
  name?: string;
  [key: string]: unknown;
}

/**
 * Builds the dynamic "Vaults" command list from the vault registry.
 *
 * `ids` only (no names) still produces commands — the palette can render
 * the raw id — while `name` improves search and display.
 */
export function buildVaultCommands(vaults: VaultRegistryItem[]): VaultCommand[] {
  return vaults
    .filter((v) => v && typeof v.id === "string" && v.id.trim().length > 0)
    .map((v) => ({
      kind: "vault" as const,
      id: `vault-${v.id}`,
      route: `/vault/${encodeURIComponent(v.id)}`,
      label: typeof v.name === "string" && v.name.trim() ? v.name : `Vault ${v.id}`,
      vaultId: v.id,
      keywords: ["vault", v.id, typeof v.name === "string" ? v.name : ""].filter(
        Boolean,
      ),
    }));
}

// ---------------------------------------------------------------------------
// Client-side actions
// ---------------------------------------------------------------------------

export const ACTION_COMMANDS: ActionCommand[] = [
  {
    kind: "action",
    id: "action-copy-wallet-address",
    actionId: "copy-wallet-address",
    labelKey: "palette.copyWalletAddress",
    keywords: ["copy", "wallet", "address", "clipboard", "account", "key"],
  },
];

// ---------------------------------------------------------------------------
// Filtering / scoring — pure helpers, unit-tested in
// `__tests__/command-palette.test.cjs`
// ---------------------------------------------------------------------------

/**
 * Returns `true` when every whitespace-separated word in `query` matches
 * somewhere in the command's searchable text (label + keywords).
 * Case-insensitive; empty query matches everything.
 */
export function commandMatches(
  entry: CommandEntry,
  query: string,
  resolveLabel: (entry: CommandEntry) => string
): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;

  const haystack = [
    resolveLabel(entry),
    entry.keywords.join(" "),
    entry.kind === "vault" ? entry.vaultId : "",
  ]
    .join(" ")
    .toLowerCase();

  return words.every((w) => haystack.includes(w));
}

/**
 * Filters and ranks entries for the palette list.
 *
 * Ranking (stable within a group):
 *  1. Prefix match on the label beats substring match (typical cmdk UX).
 *  2. Original registry order otherwise.
 */
export function filterCommands(
  entries: CommandEntry[],
  query: string,
  resolveLabel: (entry: CommandEntry) => string
): CommandEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return entries;

  return entries
    .map((entry) => {
      const label = resolveLabel(entry).toLowerCase();
      let score = 0;
      if (label.startsWith(q)) score = 2;
      else if (label.includes(q)) score = 1;
      else if (!commandMatches(entry, q, resolveLabel)) score = -1;
      return { entry, score };
    })
    .filter((r) => r.score >= 0)
    .sort((a, b) => b.score - a.score)
    .map((r) => r.entry);
}

/** Maps an entry's singular `kind` to its display group. */
const KIND_TO_GROUP: Record<CommandEntry["kind"], CommandGroup> = {
  navigation: "navigation",
  vault: "vaults",
  settings: "settings",
  action: "actions",
};

/**
 * Groups filtered entries by `CommandGroup` in display order, dropping
 * empty groups. Returns an ordered list (not a map) so callers can render
 * `cmdk` groups predictably.
 */
export function groupCommands(
  entries: CommandEntry[]
): Array<{ group: CommandGroup; entries: CommandEntry[] }> {
  const buckets = new Map<CommandGroup, CommandEntry[]>();
  for (const entry of entries) {
    const group = KIND_TO_GROUP[entry.kind];
    const list = buckets.get(group) ?? [];
    list.push(entry);
    buckets.set(group, list);
  }
  return COMMAND_GROUP_ORDER.filter((g) => buckets.has(g)).map((group) => ({
    group,
    entries: buckets.get(group)!,
  }));
}
