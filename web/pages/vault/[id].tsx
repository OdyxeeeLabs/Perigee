import { useEffect, useState } from "react";
import type { GetServerSideProps } from "next";
import Link from "next/link";
import { useTranslations } from "next-intl";
import { SEO } from "../../components/SEO";
import { useLocale } from "../../components/LocaleSync";
import { FormattedAmount, FormattedPercent } from "../../components/ui/FormattedAmount";
import { formatAmount, formatNumber } from "../../lib/locale/format";
import { Vault } from "../../types/vault";
import {
  buildVaultJsonLd,
  buildVaultOgTags,
  fetchVaultData,
} from "../../lib/vault-metadata";
import { trackFunnelEvent } from "../../lib/telemetry";

interface VaultDetailPageProps {
  vaultId: string;
  vault: Vault | null;
}

/**
 * WEB-55 (#188): the vault ID is read from the URL path via `context.params`
 * instead of `router.query`, so it is stable on first render and cannot
 * desync during client-side navigation. Vault data is fetched server-side
 * and hydrated into the page through props.
 */
export const getServerSideProps: GetServerSideProps<VaultDetailPageProps> = async (
  context,
) => {
  const rawId = context.params?.id;
  const vaultId = Array.isArray(rawId) ? rawId[0] : rawId ?? "";
  const vault = vaultId ? await fetchVaultData(vaultId) : null;

  return {
    props: {
      vaultId,
      vault,
    },
  };
};

export default function PagesVaultDetailPage({
  vaultId,
  vault,
}: VaultDetailPageProps) {
  const t = useTranslations();
  const locale = useLocale();
  const [depositOpen, setDepositOpen] = useState(false);
  const [depositAmount, setDepositAmount] = useState("100");
  const [depositSuccess, setDepositSuccess] = useState(false);

  useEffect(() => {
    if (vaultId) {
      trackFunnelEvent("vault_viewed", {
        vault_id: vaultId,
        asset: vault?.asset || "XLM",
        status: vault?.status || "UNKNOWN",
      });
    }
  }, [vaultId, vault?.asset, vault?.status]);

  const handleOpenDeposit = () => {
    setDepositOpen(true);
    setDepositSuccess(false);
    trackFunnelEvent("deposit_initiated", {
      vault_id: vaultId,
      asset: vault?.asset || "XLM",
    });
  };

  const handleSelectAmount = (amount: string) => {
    setDepositAmount(amount);
    trackFunnelEvent("deposit_amount_entered", {
      vault_id: vaultId,
      asset: vault?.asset || "XLM",
      amount_tier: Number(amount) < 100 ? "small" : Number(amount) <= 500 ? "medium" : "large",
    });
  };

  const handleConfirmDeposit = () => {
    trackFunnelEvent("deposit_submitted", {
      vault_id: vaultId,
      asset: vault?.asset || "XLM",
    });

    setTimeout(() => {
      setDepositSuccess(true);
      trackFunnelEvent("deposit_success", {
        vault_id: vaultId,
        asset: vault?.asset || "XLM",
        duration_ms: 120,
      });
    }, 400);
  };

  const jsonLd = vault ? buildVaultJsonLd(vault, vaultId) : undefined;
  const customOg = vault ? buildVaultOgTags(vault, vaultId) : undefined;
  const title = vault ? `${vault.name} | ${t("app.name")}` : t("vault.defaultTitle");
  const description = vault
    ? t("vault.metaDescription", {
        name: vault.name,
        siteName: t("app.name"),
        balance: formatAmount(vault.balance, locale),
        asset: vault.asset,
        apy: formatNumber(vault.apy ?? 0, locale, { maximumFractionDigits: 2 }),
        status: vault.status,
      })
    : t("vault.defaultDescription");

  return (
    <>
      <SEO
        title={title}
        description={description}
        path={`/vault/${vaultId}`}
        jsonLd={jsonLd}
        customOg={customOg}
      />

      <main className="min-h-screen bg-slate-950 text-slate-100">
        <header className="sticky top-0 z-40 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
            <div>
              <Link href="/" className="text-2xl font-bold text-cyan-400 hover:text-cyan-300">
                {t("app.name")}
              </Link>
              <span className="ml-3 text-xs uppercase tracking-wider text-slate-500">
                {t("vault.details")}
              </span>
            </div>
            <Link
              href="/"
              className="text-sm font-medium text-slate-400 hover:text-cyan-400 transition-colors"
            >
              {t("nav.backToDashboard")}
            </Link>
          </div>
        </header>

        <div className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
          {!vault ? (
            <div className="py-12 text-center text-slate-400">{t("vault.notFound")}</div>
          ) : (
            <>
              {/* Breadcrumb */}
              <nav className="mb-6 flex items-center gap-2 text-xs text-slate-500">
                <Link href="/" className="hover:text-slate-400">{t("nav.home")}</Link>
                <span>/</span>
                <span className="text-slate-400">{t("nav.vaults")}</span>
                <span>/</span>
                <span className="font-mono text-cyan-400">{vault.id}</span>
              </nav>

              {/* Vault Title & Status */}
              <div className="flex flex-col gap-4 border-b border-slate-800 pb-6 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <div className="flex items-center gap-3">
                    <h1 className="text-3xl font-bold text-white">{vault.name}</h1>
                    <span
                      className={`inline-block rounded-full px-3 py-1 text-xs font-semibold ${
                        vault.status === "ACTIVE"
                          ? "bg-emerald-950 text-emerald-400 border border-emerald-800"
                          : "bg-amber-950 text-amber-400 border border-amber-800"
                      }`}
                    >
                      {vault.status}
                    </span>
                  </div>
                  <p className="mt-1 font-mono text-xs text-slate-400">
                    {t("vault.vaultId", { id: vault.id })}
                  </p>
                </div>

                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    onClick={handleOpenDeposit}
                    className="rounded-lg bg-cyan-600 px-4 py-2 text-sm font-semibold text-white shadow-lg shadow-cyan-900/30 hover:bg-cyan-500 transition-colors"
                  >
                    Deposit Funds
                  </button>
                </div>
              </div>

              {/* Key Metrics Cards */}
              <div className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.totalBalance")}</p>
                  <p className="mt-2 text-2xl font-bold text-white">
                    <FormattedAmount value={vault.balance} locale={locale} />{" "}
                    <span className="text-sm font-normal text-cyan-400">{vault.asset}</span>
                  </p>
                </div>

                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.projectedApy")}</p>
                  <p className="mt-2 text-2xl font-bold text-emerald-400">
                    <FormattedPercent
                      value={vault.apy ?? 0}
                      locale={locale}
                      digits={2}
                    />
                  </p>
                </div>

                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.underlyingAsset")}</p>
                  <p className="mt-2 text-2xl font-bold text-white">{vault.asset || "XLM"}</p>
                </div>

                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.managerOwner")}</p>
                  <p className="mt-2 truncate font-mono text-sm font-medium text-slate-300">
                    {vault.owner}
                  </p>
                </div>
              </div>
            </>
          )}
        </div>

        {/* Deposit Modal with Funnel Tracking */}
        {depositOpen && vault && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4">
            <div className="w-full max-w-md rounded-2xl border border-slate-800 bg-slate-900 p-6 shadow-2xl text-slate-100">
              <h3 className="text-xl font-bold text-white mb-2">Deposit to {vault.name}</h3>
              <p className="text-xs text-slate-400 mb-5">
                Allocate funds into this automated strategy vault.
              </p>

              {depositSuccess ? (
                <div className="rounded-xl border border-emerald-800/80 bg-emerald-950/40 p-5 text-center">
                  <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-emerald-900/60 text-emerald-400 font-bold mb-2">
                    ✓
                  </div>
                  <p className="font-semibold text-emerald-300">Deposit Completed Successfully</p>
                  <p className="text-xs text-slate-400 mt-1">
                    {depositAmount} {vault.asset} deposited to vault.
                  </p>
                  <button
                    type="button"
                    onClick={() => setDepositOpen(false)}
                    className="mt-4 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-semibold text-white hover:bg-emerald-500"
                  >
                    Done
                  </button>
                </div>
              ) : (
                <>
                  <div className="mb-4">
                    <label className="text-xs font-medium text-slate-400 block mb-2">
                      Amount ({vault.asset})
                    </label>
                    <div className="grid grid-cols-3 gap-2 mb-3">
                      {["50", "100", "500"].map((amt) => (
                        <button
                          key={amt}
                          type="button"
                          onClick={() => handleSelectAmount(amt)}
                          className={`rounded-lg border py-2 text-xs font-medium transition-colors ${
                            depositAmount === amt
                              ? "border-cyan-500 bg-cyan-950/60 text-cyan-300"
                              : "border-slate-800 bg-slate-950 hover:bg-slate-800 text-slate-300"
                          }`}
                        >
                          {amt} {vault.asset}
                        </button>
                      ))}
                    </div>
                    <input
                      type="number"
                      value={depositAmount}
                      onChange={(e) => handleSelectAmount(e.target.value)}
                      className="w-full rounded-lg border border-slate-800 bg-slate-950 px-3 py-2 text-sm text-white focus:border-cyan-500 focus:outline-none"
                    />
                  </div>

                  <div className="flex justify-end gap-3 mt-6">
                    <button
                      type="button"
                      onClick={() => setDepositOpen(false)}
                      className="rounded-lg border border-slate-800 px-4 py-2 text-xs font-medium text-slate-400 hover:bg-slate-800 hover:text-white"
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={handleConfirmDeposit}
                      className="rounded-lg bg-cyan-600 px-5 py-2 text-xs font-semibold text-white hover:bg-cyan-500 shadow-md shadow-cyan-900/40"
                    >
                      Confirm Deposit
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
        )}
      </main>
    </>
  );
}
