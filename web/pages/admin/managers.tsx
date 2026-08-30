"use client";

import Head from "next/head";
import { useRouter } from "next/router";
import { useState, useEffect } from "react";
import { ConnectButton } from "../../components/ConnectButton";
import { SEO } from "../../components/SEO";
import { managerService, type ManagerRecord } from "../../lib/api";

export default function AdminManagers() {
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
        err instanceof Error ? err.message : "Failed to load managers";
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
      const msg = err instanceof Error ? err.message : "Approval failed";
      alert(msg);
    } finally {
      setActionLoading(null);
    }
  }

  async function handleReject(id: string) {
    const notes = prompt("Rejection reason (optional):");
    if (notes === null) return;
    setActionLoading(id);
    try {
      await managerService.reject(id, notes);
      await loadManagers();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Rejection failed";
      alert(msg);
    } finally {
      setActionLoading(null);
    }
  }

  return (
    <>
      <Head />
      <SEO
        title="Admin — Managers"
        description="Approve or reject wealth-manager onboarding submissions for the Perigee autonomous portfolio protocol."
        path="/admin/managers"
        noIndex
      />
      <main className="min-h-screen bg-slate-950 text-slate-100">
        <header className="sticky top-0 z-50 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
            <div>
              <h1 className="text-2xl font-bold text-cyan-400">Perigee</h1>
              <p className="text-sm text-slate-400">Admin — Manager Approval</p>
            </div>
            <ConnectButton />
          </div>
        </header>

        <section className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
          <div className="mb-6 flex items-center gap-3">
            {["", "pending", "approved", "rejected"].map((s) => (
              <button
                key={s}
                onClick={() => setFilter(s)}
                className={`rounded-lg px-4 py-2 text-sm font-medium transition-colors ${
                  filter === s
                    ? "bg-cyan-600 text-white"
                    : "bg-slate-800 text-slate-400 hover:bg-slate-700"
                }`}
              >
                {s || "All"}
              </button>
            ))}
          </div>

          {loading ? (
            <div className="flex flex-col items-center justify-center py-12">
              <div
                className="h-8 w-8 animate-spin rounded-full border-4 border-slate-700 border-t-cyan-400"
                aria-label="Loading managers"
              />
              <p className="mt-3 text-sm text-slate-500">Loading managers...</p>
            </div>
          ) : error ? (
            <div className="rounded-lg border border-red-800 bg-red-950/40 p-4 text-center">
              <p className="text-red-400">{error}</p>
              <button
                onClick={loadManagers}
                className="mt-3 rounded-lg bg-cyan-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-cyan-500"
              >
                Retry
              </button>
            </div>
          ) : managers.length === 0 ? (
            <div className="py-12 text-center">
              <p className="text-slate-400">
                No {filter ? `${filter} ` : ""}managers found.
              </p>
              <p className="mt-1 text-sm text-slate-600">
                Managers will appear here when they are available.
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto rounded-2xl border border-slate-800">
              <table className="w-full text-sm">
                <thead className="bg-slate-900">
                  <tr>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Name
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Stellar Address
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Email
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Status
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      KYC Ref
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Notes
                    </th>
                    <th className="px-4 py-3 text-left font-medium text-slate-400">
                      Actions
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
                            <button
                              onClick={() => handleApprove(m.id)}
                              disabled={actionLoading === m.id}
                              className="rounded bg-green-700 px-3 py-1 text-xs font-medium text-white hover:bg-green-600 disabled:opacity-50 transition-colors"
                            >
                              {actionLoading === m.id ? "..." : "Approve"}
                            </button>
                            <button
                              onClick={() => handleReject(m.id)}
                              disabled={actionLoading === m.id}
                              className="rounded bg-red-700 px-3 py-1 text-xs font-medium text-white hover:bg-red-600 disabled:opacity-50 transition-colors"
                            >
                              Reject
                            </button>
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
            <button
              onClick={() => router.push("/")}
              className="text-sm text-cyan-400 hover:text-cyan-300 transition-colors"
            >
              &larr; Back to Analyzer
            </button>
          </div>
        </section>
      </main>
    </>
  );
}
