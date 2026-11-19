"use client";

/**
 * Manager dashboard — App Router page.
 *
 * Replaces the previous placeholder with a live integration against
 * GET /api/v1/managers via `managerService.list()`.
 *
 * Renders four distinct UI states:
 *  - Loading   — spinner while the request is in-flight
 *  - Error     — inline error card with a Retry action
 *  - Empty     — descriptive empty state when no managers exist
 *  - Populated — table of ManagerRecord rows
 *
 * No mock data is used; in production mode the component always
 * fetches from the live API.
 */

import { useEffect, useState, useCallback } from "react";
import Link from "next/link";
import { managerService, type ManagerRecord, ApiError } from "@/lib/api";

// ── Status badge ──────────────────────────────────────────────────────────────

function StatusBadge({ status }: { status: string }) {
  const colour =
    status === "approved"
      ? "bg-emerald-900/50 text-emerald-400 border-emerald-800"
      : status === "rejected"
        ? "bg-red-900/50 text-red-400 border-red-800"
        : "bg-yellow-900/50 text-yellow-400 border-yellow-800";

  return (
    <span
      className={`inline-block rounded-full border px-2.5 py-0.5 text-xs font-medium capitalize ${colour}`}
    >
      {status}
    </span>
  );
}

// ── Loading skeleton ──────────────────────────────────────────────────────────

function LoadingState() {
  return (
    <div
      className="flex flex-col items-center justify-center py-20"
      role="status"
      aria-live="polite"
      aria-label="Loading managers"
    >
      <div className="h-9 w-9 animate-spin rounded-full border-4 border-slate-700 border-t-cyan-400" />
      <p className="mt-4 text-sm text-slate-500">Loading managers…</p>
    </div>
  );
}

// ── Error state ───────────────────────────────────────────────────────────────

function ErrorState({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      className="rounded-xl border border-red-800 bg-red-950/40 p-8 text-center"
      role="alert"
    >
      <svg
        className="mx-auto mb-3 h-8 w-8 text-red-500"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
        aria-hidden="true"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 9v2m0 4h.01M12 3a9 9 0 110 18A9 9 0 0112 3z"
        />
      </svg>
      <p className="text-red-300">{message}</p>
      <button
        type="button"
        onClick={onRetry}
        className="mt-4 rounded-lg bg-red-800 px-4 py-2 text-sm font-medium text-red-100 hover:bg-red-700 transition-colors"
      >
        Retry
      </button>
    </div>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function EmptyState({ filter }: { filter: string }) {
  return (
    <div className="py-20 text-center">
      <svg
        className="mx-auto mb-4 h-12 w-12 text-slate-700"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
        aria-hidden="true"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.5}
          d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0"
        />
      </svg>
      <p className="text-slate-400">
        No{filter ? ` ${filter}` : ""} managers found.
      </p>
      <p className="mt-1 text-sm text-slate-600">
        Managers will appear here once they submit onboarding requests.
      </p>
      <Link
        href="/managers/onboarding"
        className="mt-5 inline-block rounded-lg bg-cyan-700 px-4 py-2 text-sm font-medium text-white hover:bg-cyan-600 transition-colors"
      >
        Onboard a manager
      </Link>
    </div>
  );
}

// ── Manager row ───────────────────────────────────────────────────────────────

function ManagerRow({ manager }: { manager: ManagerRecord }) {
  return (
    <tr className="border-t border-slate-800 hover:bg-slate-900/50 transition-colors">
      <td className="px-4 py-3 font-medium text-slate-200">{manager.name}</td>
      <td className="px-4 py-3 font-mono text-xs text-slate-400">
        {manager.stellar_address}
      </td>
      <td className="px-4 py-3 text-slate-400">{manager.email || "—"}</td>
      <td className="px-4 py-3">
        <StatusBadge status={manager.status} />
      </td>
      <td className="px-4 py-3 font-mono text-xs text-slate-500">
        {manager.kyc_document_ref || "—"}
      </td>
      <td className="px-4 py-3 text-xs text-slate-500">
        {manager.notes || "—"}
      </td>
      <td className="px-4 py-3 text-xs text-slate-600">
        {new Date(manager.created_at).toLocaleDateString(undefined, {
          year: "numeric",
          month: "short",
          day: "numeric",
        })}
      </td>
    </tr>
  );
}

// ── Filter tabs ───────────────────────────────────────────────────────────────

const FILTER_OPTIONS = [
  { value: "", label: "All" },
  { value: "pending", label: "Pending" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
];

// ── Page component ────────────────────────────────────────────────────────────

export default function ManagerDashboardPage() {
  const [managers, setManagers] = useState<ManagerRecord[]>([]);
  const [filter, setFilter] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchManagers = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const records = await managerService.list(filter || undefined);
      setManagers(records);
    } catch (err) {
      const message =
        err instanceof ApiError
          ? `API Error ${err.status}: ${err.message}`
          : err instanceof Error
            ? err.message
            : "An unexpected error occurred while loading managers.";
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [filter]);

  useEffect(() => {
    fetchManagers();
  }, [fetchManagers]);

  return (
    <main className="min-h-screen bg-slate-950 text-slate-100">
      {/* Header */}
      <header className="sticky top-0 z-50 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
          <div>
            <Link
              href="/"
              className="text-2xl font-bold text-cyan-400 hover:text-cyan-300 transition-colors"
            >
              Perigee
            </Link>
            <p className="text-xs text-slate-500 mt-0.5">Manager Dashboard</p>
          </div>
          <Link
            href="/managers/onboarding"
            className="rounded-lg bg-cyan-700 px-4 py-2 text-sm font-medium text-white hover:bg-cyan-600 transition-colors"
          >
            + Onboard Manager
          </Link>
        </div>
      </header>

      <section className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
        {/* Page title */}
        <div className="mb-6">
          <h1 className="text-2xl font-bold text-white">Managers</h1>
          <p className="mt-1 text-sm text-slate-500">
            Wealth managers provisioned on the Perigee protocol.
          </p>
        </div>

        {/* Filter tabs */}
        <div
          className="mb-6 flex flex-wrap gap-2"
          role="tablist"
          aria-label="Filter managers by status"
        >
          {FILTER_OPTIONS.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={filter === value}
              onClick={() => setFilter(value)}
              className={`rounded-full px-4 py-1.5 text-sm font-medium transition-colors ${
                filter === value
                  ? "bg-cyan-700 text-white"
                  : "bg-slate-800 text-slate-400 hover:bg-slate-700 hover:text-slate-200"
              }`}
            >
              {label}
            </button>
          ))}
        </div>

        {/* Content area */}
        {loading ? (
          <LoadingState />
        ) : error ? (
          <ErrorState message={error} onRetry={fetchManagers} />
        ) : managers.length === 0 ? (
          <EmptyState filter={filter} />
        ) : (
          <div className="overflow-x-auto rounded-xl border border-slate-800">
            <table className="w-full text-sm" aria-label="Manager list">
              <thead className="bg-slate-900">
                <tr>
                  {[
                    "Name",
                    "Stellar Address",
                    "Email",
                    "Status",
                    "KYC Ref",
                    "Notes",
                    "Created",
                  ].map((col) => (
                    <th
                      key={col}
                      scope="col"
                      className="px-4 py-3 text-left text-xs font-semibold uppercase tracking-wide text-slate-400"
                    >
                      {col}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {managers.map((m) => (
                  <ManagerRow key={m.id} manager={m} />
                ))}
              </tbody>
            </table>
          </div>
        )}

        {/* Back link */}
        <div className="mt-8">
          <Link
            href="/"
            className="text-sm text-slate-500 hover:text-cyan-400 transition-colors"
          >
            &larr; Back to Dashboard
          </Link>
        </div>
      </section>
    </main>
  );
}
