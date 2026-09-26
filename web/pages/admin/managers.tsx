"use client";

import { useRouter } from "next/router";
import { useState, useEffect } from "react";
import { useTranslations } from "next-intl";
import { ConnectButton } from "../../components/ConnectButton";
import { SEO } from "../../components/SEO";
import { Button } from "../../components/ui/Button";
import { managerService, type ManagerRecord } from "../../lib/api";

export default function AdminManagers() {
  const t = useTranslations();
  const router = useRouter();
  const [managers, setManagers] = useState<ManagerRecord[]>([]);
  const [filter, setFilter] = useState<string>("pending");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  useEffect(() => {
    loadManagers();
  }, [filter]);

  async function loadManagers() {
    setLoading(true);
    setError(null);
    try {
      const records = await managerService.list(filter || undefined);
      setManagers(records);
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : t("admin.managers.loadFailed");
      setError(msg);
    } finally {
      setLoading(false);
    }
  }

  async function handleApprove(id: string) {
    setActionLoading(id);
    try {
      await managerService.approve(id);
      await loadManagers();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("admin.managers.approvalFailed");
      alert(msg);
    } finally {
      setActionLoading(null);
    }
  }

  async function handleReject(id: string) {
    const notes = prompt(t("admin.managers.rejectionPrompt"));
    if (notes === null) return;
    setActionLoading(id);
    try {
      await managerService.reject(id, notes);
      await loadManagers();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("admin.managers.rejectionFailed");
      alert(msg);
    } finally {
      setActionLoading(null);
    }
  }

  const filterLabels: Record<string, string> = {
    "": t("admin.managers.filterAll"),
    pending: t("admin.managers.filterPending"),
    approved: t("admin.managers.filterApproved"),
    rejected: t("admin.managers.filterRejected"),
  };

  return (
    <>
      <SEO
        title={t("admin.managers.title")}
        description={t("admin.managers.description")}
        path="/admin/managers"
        noIndex
      />
      <main className="min-h-screen bg-slate-950 text-slate-100">
        <header className="sticky top-0 z-50 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
            <div>
              <h1 className="text-2xl font-bold text-cyan-400">{t("app.name")}</h1>
              <p className="text-sm text-slate-400">{t("admin.managers.subtitle")}</p>
            </div>
            <ConnectButton />
          </div>
        </header>

        <section className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
          <div className="mb-6 flex items-center gap-3">
            {["", "pending", "approved", "rejected"].map((s) => (
              <Button
                key={s}
                onClick={() => setFilter(s)}
                variant={filter === s ? "default" : "secondary"}
              >
                {filterLabels[s] || s}
              </Button>
            ))}
          </div>
          {loading ? (
            <div className="flex flex-col items-center justify-center py-12">
              <div
                className="h-8 w-8 animate-spin rounded-full border-4 border-slate-700 border-t-cyan-400"
                aria-label={t("admin.managers.loading")}
              />
              <p className="mt-3 text-sm text-slate-500">{t("admin.managers.loading")}</p>
            </div>
          ) : error ? (
            <div className="rounded-lg border border-red-800 bg-red-950/40 p-4 text-center">
              <p className="text-red-400">{error}</p>
              <Button onClick={loadManagers} className="mt-3">
                {t("admin.managers.retry")}
              </Button>
            </div>
          ) : managers.length === 0 ? (
            <div className="py-12 text-center">
              <p className="text-slate-400">
                {t("admin.managers.emptyTitle", { filter: filter ? `${filter} ` : "" })}
              </p>
              <p className="mt-1 text-sm text-slate-600">
                {t("admin.managers.emptySubtitle")}
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto rounded-2xl border border-slate-800">
              <table className="w-full text-sm">
                <thead className="bg-slate-900">
                  <tr>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colName")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colAddress")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colEmail")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colStatus")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colKyc")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colNotes")}
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      {t("admin.managers.colActions")}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-800">
                  {managers.map((m) => (
                    <tr key={m.id} className="hover:bg-slate-900/50">
                      <td className="px-4 py-3 font-medium text-slate-200">
                        {m.name}
                      </td>
                      <td className="px-4 py-3 font-mono text-xs text-slate-400">
                        {m.stellar_address}
                      </td>
                      <td className="px-4 py-3 text-slate-400">
                        {m.email || "—"}
                      </td>
                      <td className="px-4 py-3">
                        <span
                          className={`inline-block rounded-full px-2.5 py-0.5 text-xs font-medium ${
                            m.status === "approved"
                              ? "bg-green-900/50 text-green-400"
                              : m.status === "rejected"
                                ? "bg-red-900/50 text-red-400"
                                : "bg-yellow-900/50 text-yellow-400"
                          }`}
                        >
                          {m.status}
                        </span>
                      </td>
                      <td className="px-4 py-3 font-mono text-xs text-slate-500">
                        {m.kyc_document_ref || "—"}
                      </td>
                      <td className="px-4 py-3 text-xs text-slate-500">
                        {m.notes || "—"}
                      </td>
                      <td className="px-4 py-3">
                        {m.status === "pending" && (
                          <div className="flex gap-2">
                            <Button
                              onClick={() => handleApprove(m.id)}
                              disabled={actionLoading === m.id}
                              variant="default"
                              size="sm"
                            >
                              {actionLoading === m.id ? "..." : t("admin.managers.approve")}
                            </Button>
                            <Button
                              onClick={() => handleReject(m.id)}
                              disabled={actionLoading === m.id}
                              variant="destructive"
                              size="sm"
                            >
                              {t("admin.managers.reject")}
                            </Button>
                          </div>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <div className="mt-6">
            <Button variant="link" onClick={() => router.push("/")}>
              {t("nav.backToAnalyzer")}
            </Button>
          </div>
        </section>
      </main>
    </>
  );
}
