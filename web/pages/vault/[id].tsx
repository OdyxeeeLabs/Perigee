import type { GetServerSideProps } from "next";
import Link from "next/link";
import { useTranslations } from "next-intl";
import { SEO } from "../../components/SEO";
import { Vault } from "../../types/vault";
import {
  buildVaultJsonLd,
  buildVaultOgTags,
  fetchVaultData,
} from "../../lib/vault-metadata";

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
  const jsonLd = vault ? buildVaultJsonLd(vault, vaultId) : undefined;
  const customOg = vault ? buildVaultOgTags(vault, vaultId) : undefined;
  const title = vault ? `${vault.name} | ${t("app.name")}` : t("vault.defaultTitle");
  const description = vault
    ? t("vault.metaDescription", {
        name: vault.name,
        siteName: t("app.name"),
        balance: String(vault.balance),
        asset: vault.asset,
        apy: String(vault.apy),
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
              </div>

              {/* Key Metrics Cards */}
              <div className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.totalBalance")}</p>
                  <p className="mt-2 text-2xl font-bold text-white">
                    {vault.balance.toLocaleString()}{" "}
                    <span className="text-sm font-normal text-cyan-400">{vault.asset}</span>
                  </p>
                </div>

                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <p className="text-xs font-medium text-slate-400">{t("vault.projectedApy")}</p>
                  <p className="mt-2 text-2xl font-bold text-emerald-400">
                    {vault.apy !== undefined ? `${vault.apy}%` : "0.0%"}
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
      </main>
    </>
  );
}
