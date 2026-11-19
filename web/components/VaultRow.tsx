import type { KeyboardEvent } from "react";
import type { Vault } from "@/types/vault";
import { useLocale } from "@/components/LocaleSync";
import { FormattedAmount, FormattedPercent } from "@/components/ui/FormattedAmount";
import { PrefetchLink } from "./PrefetchLink";

export interface VaultRowProps {
  vault: Vault;
  style?: React.CSSProperties;
  /** When set, the vault name links to this page and prefetches it on hover (#691). */
  href?: string;
  /**
   * When set, the whole row becomes an interactive trigger (e.g. opening the
   * vault detail modal/drawer, #684). The row gets `role="button"` and
   * keyboard support; clicking the `href` link (if present) does not also
   * fire this, since the link's own click is stopped from bubbling.
   */
  onClick?: (vault: Vault) => void;
  rowRef?: (node: HTMLDivElement | null) => void;
  tabIndex?: number;
  onKeyDown?: (event: KeyboardEvent<HTMLDivElement>) => void;
  onFocus?: () => void;
}

export function VaultRow({
  vault,
  style,
  href,
  onClick,
  rowRef,
  tabIndex,
  onKeyDown,
  onFocus,
}: VaultRowProps) {
  const interactive = Boolean(onClick);
  const locale = useLocale();

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (onClick && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      onClick(vault);
    }
    onKeyDown?.(event);
  };

  return (
    <div
      ref={rowRef}
      style={style}
      onClick={interactive ? () => onClick?.(vault) : undefined}
      onKeyDown={interactive || onKeyDown ? handleKeyDown : undefined}
      onFocus={onFocus}
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? (tabIndex ?? 0) : tabIndex}
      aria-label={interactive ? `View details for ${vault.name}` : undefined}
      className={`flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900/60 px-4 py-3 ${
        interactive
          ? "cursor-pointer transition-colors hover:border-slate-700 hover:bg-slate-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-500"
          : ""
      }`}
    >
      <div className="flex flex-col">
        {href ? (
          <PrefetchLink
            href={href}
            onClick={(event) => event.stopPropagation()}
            className="text-sm font-medium text-white hover:text-cyan-300"
          >
            {vault.name}
          </PrefetchLink>
        ) : (
          <span className="text-sm font-medium text-white">{vault.name}</span>
        )}
        <span className="text-xs text-slate-400">{vault.owner}</span>
      </div>
      <div className="flex items-center gap-4 text-xs text-slate-400">
        <span className="font-mono text-cyan-400">
          <FormattedAmount value={vault.balance} locale={locale} asset={vault.asset} />
        </span>
        <span className="text-emerald-400">
          <FormattedPercent value={vault.apy ?? 0} locale={locale} digits={2} />
        </span>
        <span
          className={`rounded-full px-2 py-0.5 text-[10px] font-semibold ${
            vault.status === "ACTIVE"
              ? "bg-emerald-950 text-emerald-400"
              : vault.status === "LOCKED"
                ? "bg-red-950 text-red-400"
                : vault.status === "PENDING"
                  ? "bg-amber-950 text-amber-400"
                  : "bg-slate-800 text-slate-400"
          }`}
        >
          {vault.status}
        </span>
      </div>
    </div>
  );
}
