"use client";

import React, { useState, useEffect, useCallback } from "react";
import Head from "next/head";
import Link from "next/link";
import {
  Activity,
  ArrowRight,
  BarChart3,
  CheckCircle2,
  Clock,
  Download,
  Eye,
  Filter,
  Layers,
  Lock,
  Play,
  RefreshCw,
  Server,
  Shield,
  ShieldAlert,
  ShieldCheck,
  TrendingDown,
  TrendingUp,
  UserCheck,
  Wallet,
  Zap,
} from "lucide-react";
import { SEO } from "../../components/SEO";
import { useDateRange } from "../../hooks/useDateRange";
import { DATE_RANGE_PRESETS, dateRangeToApiParams } from "../../lib/dateRange";
import {
  trackTelemetryEvent,
  trackFunnelEvent,
  getTelemetryConsent,
  optOutTelemetry,
  optInTelemetry,
  isDoNotTrackEnabled,
} from "../../lib/telemetry";
import type { TelemetrySummary } from "../api/telemetry";

export default function AnalyticsDashboardPage() {
  const [summary, setSummary] = useState<TelemetrySummary | null>(null);
  const [loading, setLoading] = useState(true);
  const {
    range: timeframe,
    setRange: setTimeframe,
    label: rangeLabel,
  } = useDateRange();
  const [selectedCategory, setSelectedCategory] = useState<string>("all");
  const [testEventSent, setTestEventSent] = useState(false);
  const [optOutStatus, setOptOutStatus] = useState<boolean | null>(null);
  const [isDnt, setIsDnt] = useState(false);
  const [activeTab, setActiveTab] = useState<"overview" | "funnel" | "events" | "providers" | "compliance">("overview");

  const fetchTelemetryData = useCallback(async () => {
    setLoading(true);
    try {
      const params = new URLSearchParams(dateRangeToApiParams(timeframe));
      const res = await fetch(`/api/telemetry?${params.toString()}`);
      if (res.ok) {
        const data = (await res.json()) as TelemetrySummary;
        setSummary(data);
      }
    } catch {
      // Backend collector fallback
    } finally {
      setLoading(false);
    }
  }, [timeframe]);

  useEffect(() => {
    fetchTelemetryData();
    setOptOutStatus(getTelemetryConsent());
    setIsDnt(isDoNotTrackEnabled());
  }, [fetchTelemetryData]);

  const handleTriggerTestEvent = () => {
    // Send a sample funnel flow to verify pipeline end-to-end
    trackFunnelEvent("wallet_connect", { wallet_id: "freighter" });
    trackFunnelEvent("vault_viewed", { vault_id: "xlm-yield-v1", asset: "XLM", status: "ACTIVE" });
    trackFunnelEvent("deposit_initiated", { vault_id: "xlm-yield-v1", asset: "XLM" });
    trackFunnelEvent("deposit_success", { vault_id: "xlm-yield-v1", asset: "XLM", duration_ms: 95 });
    trackTelemetryEvent({
      name: "contract_analyze",
      properties: { mode: "wasm", fn: "yield_accrue" },
    });

    setTestEventSent(true);
    setTimeout(() => {
      setTestEventSent(false);
      fetchTelemetryData();
    }, 800);
  };

  const handleToggleOptOut = () => {
    if (optOutStatus === true) {
      optOutTelemetry();
      setOptOutStatus(false);
    } else {
      optInTelemetry({ performance: true, errors: true, analytics: true });
      setOptOutStatus(true);
    }
    fetchTelemetryData();
  };

  const handleExportData = () => {
    if (!summary) return;
    const jsonString = `data:text/json;charset=utf-8,${encodeURIComponent(
      JSON.stringify(summary, null, 2),
    )}`;
    const downloadAnchor = document.createElement("a");
    downloadAnchor.setAttribute("href", jsonString);
    downloadAnchor.setAttribute("download", `perigee-analytics-${timeframe}-${Date.now()}.json`);
    document.body.appendChild(downloadAnchor);
    downloadAnchor.click();
    downloadAnchor.remove();
  };

  const funnel = summary?.funnel || {
    walletConnect: 0,
    vaultView: 0,
    deposit: 0,
    walletToVaultRate: 0,
    vaultToDepositRate: 0,
    overallConversionRate: 0,
  };

  const filteredEvents = (summary?.recentEvents || []).filter((e) => {
    if (selectedCategory === "all") return true;
    return (e.category || "feature") === selectedCategory;
  });

  return (
    <>
      <SEO
        title="Team Analytics & Telemetry Dashboard | Perigee"
        description="Privacy-first telemetry, funnel conversion rates, feature usage, and GDPR compliance insights for the Perigee engineering team."
        path="/analytics"
      />

      <Head>
        <meta name="robots" content="noindex,nofollow" />
      </Head>

      <main className="min-h-screen bg-slate-950 text-slate-100 pb-16">
        {/* Top Header */}
        <header className="sticky top-0 z-40 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-2 px-4 py-4 sm:px-6 lg:px-8">
            <div className="flex flex-wrap items-center gap-2 sm:gap-3">
              <Link href="/" className="text-xl font-bold tracking-tight text-cyan-400 hover:text-cyan-300">
                Perigee
              </Link>
              <span className="text-slate-600">/</span>
              <div className="flex items-center gap-2">
                <BarChart3 className="h-5 w-5 text-cyan-400" />
                <h1 className="text-base font-semibold text-white">Product Telemetry & Analytics</h1>
              </div>
            </div>

            <div className="flex items-center gap-2 sm:gap-3">
              <button
                type="button"
                onClick={handleTriggerTestEvent}
                className="flex items-center gap-1.5 rounded-lg border border-cyan-800 bg-cyan-950/50 px-3 py-1.5 text-xs font-medium text-cyan-300 hover:bg-cyan-900/60 transition-colors"
                title="Send mock funnel and feature events to verify data ingestion"
              >
                <Play className="h-3.5 w-3.5 text-cyan-400" />
                <span>{testEventSent ? "Sending..." : "Test Ingestion"}</span>
              </button>

              <button
                type="button"
                onClick={handleExportData}
                className="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-800 transition-colors"
              >
                <Download className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">Export JSON</span>
              </button>

              <button
                type="button"
                onClick={fetchTelemetryData}
                className="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-800 transition-colors"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin text-cyan-400" : ""}`} />
                <span className="hidden sm:inline">Refresh</span>
              </button>
            </div>
          </div>
        </header>

        <div className="mx-auto max-w-7xl px-4 py-8 sm:px-6 lg:px-8">
          {/* Status Banners */}
          <div className="mb-6 grid grid-cols-1 gap-3 sm:grid-cols-3">
            <div className="flex items-center gap-3 rounded-xl border border-emerald-900/60 bg-emerald-950/30 p-3.5 text-xs text-emerald-300">
              <ShieldCheck className="h-5 w-5 shrink-0 text-emerald-400" />
              <div>
                <p className="font-semibold text-emerald-200">Zero PII Active</p>
                <p className="text-emerald-400/80">Stellar keys, IPs, and emails automatically redacted.</p>
              </div>
            </div>

            <div className="flex items-center gap-3 rounded-xl border border-cyan-900/60 bg-cyan-950/30 p-3.5 text-xs text-cyan-300">
              <Shield className="h-5 w-5 shrink-0 text-cyan-400" />
              <div>
                <p className="font-semibold text-cyan-200">GDPR & CCPA Compliant</p>
                <p className="text-cyan-400/80">Cookie-free, 30-day consent TTL, instant opt-out enabled.</p>
              </div>
            </div>

            <div className="flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/50 p-3.5 text-xs text-slate-300">
              <Lock className="h-5 w-5 shrink-0 text-slate-400" />
              <div>
                <p className="font-semibold text-slate-200">
                  {isDnt ? "GPC / DNT Signal Enforced" : "Opt-Out Mechanism Ready"}
                </p>
                <p className="text-slate-400">
                  {isDnt ? "Browser privacy signal active." : "Users can revoke consent at any time."}
                </p>
              </div>
            </div>
          </div>

          {/* Timeframe & Navigation Tabs */}
          <div className="mb-6 flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between border-b border-slate-800 pb-4">
            {/* View Tabs */}
            <div className="flex items-center gap-1 overflow-x-auto pb-1 sm:pb-0">
              {(
                [
                  { id: "overview", label: "Overview", icon: Activity },
                  { id: "funnel", label: "Funnel Analysis", icon: Layers },
                  { id: "events", label: "Live Event Stream", icon: Zap },
                  { id: "providers", label: "Providers & Drivers", icon: Server },
                  { id: "compliance", label: "Privacy & GDPR", icon: ShieldCheck },
                ] as const
              ).map((tab) => {
                const Icon = tab.icon;
                return (
                  <button
                    key={tab.id}
                    type="button"
                    onClick={() => setActiveTab(tab.id)}
                    className={`flex items-center gap-2 rounded-lg px-3.5 py-2 text-xs font-medium transition-colors whitespace-nowrap ${
                      activeTab === tab.id
                        ? "bg-cyan-950 border border-cyan-700/80 text-cyan-300"
                        : "text-slate-400 hover:bg-slate-900 hover:text-slate-200"
                    }`}
                  >
                    <Icon className="h-4 w-4" />
                    <span>{tab.label}</span>
                  </button>
                );
              })}
            </div>

            <div className="flex min-w-0 flex-col items-start gap-1.5 self-start sm:self-auto">
              <div
                role="group"
                aria-label="Comparison date range"
                className="flex flex-wrap items-center gap-1.5"
              >
                {DATE_RANGE_PRESETS.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => setTimeframe(preset.id)}
                    aria-pressed={timeframe === preset.id}
                    data-testid={`range-${preset.id}`}
                    className={`rounded-lg px-3 py-1.5 text-xs font-semibold uppercase tracking-wider transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 ${
                      timeframe === preset.id
                        ? "bg-slate-800 text-white border border-slate-700"
                        : "text-slate-400 hover:text-slate-200"
                    }`}
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
              <p className="text-[11px] text-slate-500" data-testid="range-label">
                {rangeLabel}
              </p>
            </div>
          </div>

          {/* ── TAB 1: OVERVIEW ──────────────────────────────────────────────── */}
          {activeTab === "overview" && (
            <div className="space-y-6">
              {/* KPI Cards */}
              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-5">
                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>Total Events</span>
                    <Activity className="h-4 w-4 text-cyan-400" />
                  </div>
                  <p className="mt-2 text-3xl font-bold text-white">
                    {summary?.sampleCount?.toLocaleString() || 0}
                  </p>
                  <p className="mt-1 text-[11px] text-slate-500">In-memory circular buffer</p>
                </div>

                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>Unique Sessions</span>
                    <UserCheck className="h-4 w-4 text-emerald-400" />
                  </div>
                  <p className="mt-2 text-3xl font-bold text-emerald-400">
                    {summary?.uniqueSessions?.toLocaleString() || 0}
                  </p>
                  <p className="mt-1 text-[11px] text-slate-500">Zero cookies / rotating hash</p>
                </div>

                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>Funnel Conversion</span>
                    <TrendingUp className="h-4 w-4 text-cyan-400" />
                  </div>
                  <p className="mt-2 text-3xl font-bold text-white">
                    {funnel.overallConversionRate}%
                  </p>
                  <p className="mt-1 text-[11px] text-cyan-400/80">Connect → Vault → Deposit</p>
                </div>

                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>Error Events</span>
                    <ShieldAlert className="h-4 w-4 text-amber-400" />
                  </div>
                  <p className="mt-2 text-3xl font-bold text-amber-400">
                    {summary?.categoryCounts?.error || 0}
                  </p>
                  <p className="mt-1 text-[11px] text-slate-500">Sanitized error traces</p>
                </div>

                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>Performance Vitals</span>
                    <Zap className="h-4 w-4 text-emerald-400" />
                  </div>
                  <p className="mt-2 text-3xl font-bold text-white">
                    {summary?.categoryCounts?.performance || 0}
                  </p>
                  <p className="mt-1 text-[11px] text-slate-500">LCP, INP, CLS, TTFB, Hydration</p>
                </div>
              </div>

              {/* Primary Funnel Summary Widget */}
              <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 mb-6">
                  <div>
                    <h2 className="text-lg font-bold text-white">Funnel Conversion: Connect → View → Deposit</h2>
                    <p className="text-xs text-slate-400 mt-0.5">
                      Monitors user journey progression from wallet connection to final capital deposit.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="rounded-full bg-cyan-950 px-3 py-1 text-xs font-semibold text-cyan-400 border border-cyan-800">
                      Overall: {funnel.overallConversionRate}%
                    </span>
                  </div>
                </div>

                {/* 3-Step Funnel Flow */}
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                  {/* Step 1 */}
                  <div className="relative rounded-xl border border-slate-800 bg-slate-950/70 p-5">
                    <div className="flex items-center justify-between mb-3">
                      <div className="flex items-center gap-2">
                        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-cyan-950 text-cyan-400 text-xs font-bold border border-cyan-800">
                          1
                        </span>
                        <h3 className="font-semibold text-white text-sm">Connect Wallet</h3>
                      </div>
                      <Wallet className="h-4 w-4 text-cyan-400" />
                    </div>
                    <p className="text-2xl font-bold text-white">{funnel.walletConnect}</p>
                    <p className="text-xs text-slate-500 mt-1">Unique wallet connections</p>
                    <div className="mt-4 pt-3 border-t border-slate-900 flex items-center justify-between text-xs">
                      <span className="text-slate-400">Step 1 → 2 Rate</span>
                      <span className="font-semibold text-cyan-300">{funnel.walletToVaultRate}%</span>
                    </div>
                  </div>

                  {/* Step 2 */}
                  <div className="relative rounded-xl border border-slate-800 bg-slate-950/70 p-5">
                    <div className="flex items-center justify-between mb-3">
                      <div className="flex items-center gap-2">
                        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-cyan-950 text-cyan-400 text-xs font-bold border border-cyan-800">
                          2
                        </span>
                        <h3 className="font-semibold text-white text-sm">View Vault</h3>
                      </div>
                      <Eye className="h-4 w-4 text-cyan-400" />
                    </div>
                    <p className="text-2xl font-bold text-white">{funnel.vaultView}</p>
                    <p className="text-xs text-slate-500 mt-1">Vault details viewed</p>
                    <div className="mt-4 pt-3 border-t border-slate-900 flex items-center justify-between text-xs">
                      <span className="text-slate-400">Step 2 → 3 Rate</span>
                      <span className="font-semibold text-cyan-300">{funnel.vaultToDepositRate}%</span>
                    </div>
                  </div>

                  {/* Step 3 */}
                  <div className="relative rounded-xl border border-emerald-900/60 bg-emerald-950/20 p-5">
                    <div className="flex items-center justify-between mb-3">
                      <div className="flex items-center gap-2">
                        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-900 text-emerald-400 text-xs font-bold border border-emerald-800">
                          3
                        </span>
                        <h3 className="font-semibold text-white text-sm">Deposit Funds</h3>
                      </div>
                      <CheckCircle2 className="h-4 w-4 text-emerald-400" />
                    </div>
                    <p className="text-2xl font-bold text-emerald-400">{funnel.deposit}</p>
                    <p className="text-xs text-slate-500 mt-1">Deposits completed</p>
                    <div className="mt-4 pt-3 border-t border-emerald-900/60 flex items-center justify-between text-xs">
                      <span className="text-slate-400">Total Funnel Success</span>
                      <span className="font-semibold text-emerald-300">{funnel.overallConversionRate}%</span>
                    </div>
                  </div>
                </div>
              </div>

              {/* Feature Usage & Error Log Grid */}
              <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                {/* Feature Usage */}
                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                  <div className="flex items-center justify-between mb-4">
                    <h3 className="font-bold text-white text-base">Top Feature Usage</h3>
                    <span className="text-xs text-slate-500">Sorted by count</span>
                  </div>

                  {(summary?.topFeatures || []).length === 0 ? (
                    <div className="py-8 text-center text-xs text-slate-500">
                      No feature usage recorded yet in this timeframe.
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {summary?.topFeatures.map((feat) => {
                        const maxCount = summary.topFeatures[0]?.count || 1;
                        const pct = Math.round((feat.count / maxCount) * 100);
                        return (
                          <div key={feat.name} className="space-y-1">
                            <div className="flex items-center justify-between text-xs">
                              <span className="font-mono text-cyan-300">{feat.name}</span>
                              <span className="font-semibold text-white">{feat.count}</span>
                            </div>
                            <div className="h-1.5 w-full rounded-full bg-slate-800 overflow-hidden">
                              <div
                                className="h-full rounded-full bg-gradient-to-r from-cyan-500 to-emerald-400 transition-all duration-500"
                                style={{ width: `${pct}%` }}
                              />
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>

                {/* Sanitized Error Trace Monitor */}
                <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                  <div className="flex items-center justify-between mb-4">
                    <h3 className="font-bold text-white text-base">Recent Errors (Sanitized)</h3>
                    <span className="rounded-full bg-slate-800 px-2.5 py-0.5 text-[10px] text-slate-400">
                      Zero PII
                    </span>
                  </div>

                  {(summary?.recentErrors || []).length === 0 ? (
                    <div className="py-8 text-center text-xs text-emerald-400">
                      ✓ No errors recorded in this timeframe. System healthy.
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {summary?.recentErrors.map((err, idx) => (
                        <div
                          key={idx}
                          className="rounded-xl border border-red-950/60 bg-red-950/20 p-3 text-xs"
                        >
                          <div className="flex items-center justify-between mb-1">
                            <span className="font-mono text-[11px] font-semibold text-red-300 truncate max-w-[200px]">
                              {err.component || "Global error"}
                            </span>
                            <span className="rounded bg-red-900/60 px-2 py-0.5 text-[10px] text-red-200">
                              {err.count} {err.count === 1 ? "occurrence" : "occurrences"}
                            </span>
                          </div>
                          <p className="text-slate-300 font-mono text-[11px] break-all">{err.message}</p>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          {/* ── TAB 2: FUNNEL ANALYSIS ────────────────────────────────────────── */}
          {activeTab === "funnel" && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                <h2 className="text-lg font-bold text-white mb-2">Funnel Conversion Pipeline</h2>
                <p className="text-xs text-slate-400 mb-6">
                  Detailed conversion drop-off analysis across the core product lifecycle.
                </p>

                <div className="space-y-6">
                  {/* Stage 1 */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-cyan-950 text-cyan-400 font-bold border border-cyan-800">
                          1
                        </div>
                        <div>
                          <h3 className="font-semibold text-white">Stage 1: Connect Wallet</h3>
                          <p className="text-xs text-slate-400">
                            User selects and authenticates supported Stellar wallet (Freighter, Albedo, Lobstr).
                          </p>
                        </div>
                      </div>
                      <span className="text-2xl font-bold text-white">{funnel.walletConnect}</span>
                    </div>

                    <div className="mt-4 flex items-center gap-4 text-xs text-slate-400">
                      <span>Conversion to Vault View:</span>
                      <div className="h-2 flex-1 rounded-full bg-slate-800 overflow-hidden">
                        <div
                          className="h-full bg-cyan-500 rounded-full"
                          style={{ width: `${Math.min(funnel.walletToVaultRate, 100)}%` }}
                        />
                      </div>
                      <span className="font-bold text-cyan-300">{funnel.walletToVaultRate}%</span>
                    </div>
                  </div>

                  {/* Stage 2 */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-cyan-950 text-cyan-400 font-bold border border-cyan-800">
                          2
                        </div>
                        <div>
                          <h3 className="font-semibold text-white">Stage 2: View Vault Details</h3>
                          <p className="text-xs text-slate-400">
                            User navigates to vault details page, inspects APY, asset allocation, and owner.
                          </p>
                        </div>
                      </div>
                      <span className="text-2xl font-bold text-white">{funnel.vaultView}</span>
                    </div>

                    <div className="mt-4 flex items-center gap-4 text-xs text-slate-400">
                      <span>Conversion to Deposit:</span>
                      <div className="h-2 flex-1 rounded-full bg-slate-800 overflow-hidden">
                        <div
                          className="h-full bg-emerald-500 rounded-full"
                          style={{ width: `${Math.min(funnel.vaultToDepositRate, 100)}%` }}
                        />
                      </div>
                      <span className="font-bold text-emerald-300">{funnel.vaultToDepositRate}%</span>
                    </div>
                  </div>

                  {/* Stage 3 */}
                  <div className="rounded-xl border border-emerald-900/60 bg-emerald-950/20 p-5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-emerald-900 text-emerald-400 font-bold border border-emerald-800">
                          3
                        </div>
                        <div>
                          <h3 className="font-semibold text-white">Stage 3: Deposit Capital</h3>
                          <p className="text-xs text-slate-400">
                            User triggers deposit dialog, specifies amount, and signs transaction.
                          </p>
                        </div>
                      </div>
                      <span className="text-2xl font-bold text-emerald-400">{funnel.deposit}</span>
                    </div>

                    <div className="mt-4 pt-4 border-t border-emerald-900/60 flex items-center justify-between text-xs">
                      <span className="text-slate-300 font-medium">End-to-End Funnel Conversion:</span>
                      <span className="text-lg font-bold text-emerald-300">{funnel.overallConversionRate}%</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* ── TAB 3: LIVE EVENT STREAM ─────────────────────────────────────── */}
          {activeTab === "events" && (
            <div className="space-y-4">
              {/* Category Filter Pills */}
              <div className="flex items-center gap-2 overflow-x-auto pb-2">
                <span className="text-xs text-slate-500 flex items-center gap-1 mr-1">
                  <Filter className="h-3.5 w-3.5" /> Category:
                </span>
                {(["all", "funnel", "pageview", "feature", "error", "performance"] as const).map((cat) => (
                  <button
                    key={cat}
                    type="button"
                    onClick={() => setSelectedCategory(cat)}
                    className={`rounded-lg px-3 py-1 text-xs font-medium uppercase tracking-wider transition-colors ${
                      selectedCategory === cat
                        ? "bg-cyan-600 text-white"
                        : "bg-slate-900 text-slate-400 hover:text-white border border-slate-800"
                    }`}
                  >
                    {cat}
                  </button>
                ))}
              </div>

              {/* Event Stream Table */}
              <div className="rounded-2xl border border-slate-800 bg-slate-900/60 overflow-hidden backdrop-blur">
                <div className="overflow-x-auto">
                  <table className="w-full text-left text-xs">
                    <thead className="border-b border-slate-800 bg-slate-950/80 text-slate-400">
                      <tr>
                        <th className="px-4 py-3 font-semibold">Timestamp</th>
                        <th className="px-4 py-3 font-semibold">Category</th>
                        <th className="px-4 py-3 font-semibold">Event Name</th>
                        <th className="px-4 py-3 font-semibold">Sanitized Properties (No PII)</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-800/60 font-mono">
                      {filteredEvents.length === 0 ? (
                        <tr>
                          <td colSpan={4} className="px-4 py-8 text-center text-slate-500">
                            No events matching this filter.
                          </td>
                        </tr>
                      ) : (
                        filteredEvents.map((ev, i) => (
                          <tr key={i} className="hover:bg-slate-800/30 transition-colors">
                            <td className="px-4 py-3 text-slate-400 whitespace-nowrap">
                              {ev.timestamp ? new Date(ev.timestamp).toLocaleTimeString() : "Just now"}
                            </td>
                            <td className="px-4 py-3">
                              <span
                                className={`rounded px-2 py-0.5 text-[10px] uppercase font-semibold tracking-wider ${
                                  ev.category === "funnel"
                                    ? "bg-cyan-950 text-cyan-300 border border-cyan-800"
                                    : ev.category === "error"
                                      ? "bg-red-950 text-red-300 border border-red-800"
                                      : ev.category === "performance"
                                        ? "bg-emerald-950 text-emerald-300 border border-emerald-800"
                                        : "bg-slate-800 text-slate-300"
                                }`}
                              >
                                {ev.category || "feature"}
                              </span>
                            </td>
                            <td className="px-4 py-3 font-semibold text-slate-200">
                              {ev.name}
                            </td>
                            <td className="px-4 py-3 text-slate-300">
                              {ev.properties && Object.keys(ev.properties).length > 0 ? (
                                <div className="flex flex-wrap gap-1.5">
                                  {Object.entries(ev.properties).map(([k, v]) => (
                                    <span
                                      key={k}
                                      className="rounded bg-slate-950 px-2 py-0.5 text-[11px] text-slate-300 border border-slate-800"
                                    >
                                      <span className="text-cyan-400">{k}:</span> {String(v)}
                                    </span>
                                  ))}
                                </div>
                              ) : (
                                <span className="text-slate-600">none</span>
                              )}
                            </td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </div>
            </div>
          )}

          {/* ── TAB 4: PROVIDERS & DRIVERS ────────────────────────────────────── */}
          {activeTab === "providers" && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                <h2 className="text-lg font-bold text-white mb-2">Supported Analytics Providers</h2>
                <p className="text-xs text-slate-400 mb-6">
                  Perigee supports Plausible, Umami, PostHog (self-hosted or cloud), and our built-in internal privacy collector.
                  Configure providers via environment variables in <code className="text-cyan-400">web/.env</code>.
                </p>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  {/* Plausible Card */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5">
                    <div className="flex items-center justify-between mb-2">
                      <h3 className="font-semibold text-white">1. Plausible Analytics</h3>
                      <span className="rounded bg-cyan-950 px-2 py-0.5 text-[10px] font-semibold text-cyan-300 border border-cyan-800">
                        Default / Active
                      </span>
                    </div>
                    <p className="text-xs text-slate-400 mb-4">
                      Lightweight, open-source, cookie-less privacy analytics.
                    </p>
                    <div className="rounded bg-slate-900 p-3 font-mono text-[11px] text-slate-300 space-y-1">
                      <p>NEXT_PUBLIC_ANALYTICS_PROVIDER=&quot;plausible&quot;</p>
                      <p>NEXT_PUBLIC_PLAUSIBLE_DOMAIN=&quot;perigee.app&quot;</p>
                      <p>NEXT_PUBLIC_PLAUSIBLE_HOST=&quot;https://plausible.io&quot;</p>
                    </div>
                  </div>

                  {/* Umami Card */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5">
                    <div className="flex items-center justify-between mb-2">
                      <h3 className="font-semibold text-white">2. Umami (Self-Hosted / Cloud)</h3>
                      <span className="rounded bg-slate-800 px-2 py-0.5 text-[10px] font-semibold text-slate-400">
                        Available
                      </span>
                    </div>
                    <p className="text-xs text-slate-400 mb-4">
                      Self-hosted privacy-focused alternative to Google Analytics.
                    </p>
                    <div className="rounded bg-slate-900 p-3 font-mono text-[11px] text-slate-300 space-y-1">
                      <p>NEXT_PUBLIC_ANALYTICS_PROVIDER=&quot;umami&quot;</p>
                      <p>NEXT_PUBLIC_UMAMI_WEBSITE_ID=&quot;your-website-id&quot;</p>
                      <p>NEXT_PUBLIC_UMAMI_HOST=&quot;https://analytics.umami.is&quot;</p>
                    </div>
                  </div>

                  {/* PostHog Card */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5">
                    <div className="flex items-center justify-between mb-2">
                      <h3 className="font-semibold text-white">3. Self-Hosted PostHog</h3>
                      <span className="rounded bg-slate-800 px-2 py-0.5 text-[10px] font-semibold text-slate-400">
                        Available
                      </span>
                    </div>
                    <p className="text-xs text-slate-400 mb-4">
                      Product telemetry and event pipelines deployed on your own infrastructure.
                    </p>
                    <div className="rounded bg-slate-900 p-3 font-mono text-[11px] text-slate-300 space-y-1">
                      <p>NEXT_PUBLIC_ANALYTICS_PROVIDER=&quot;posthog&quot;</p>
                      <p>NEXT_PUBLIC_POSTHOG_HOST=&quot;https://posthog.yourdomain.com&quot;</p>
                      <p>NEXT_PUBLIC_POSTHOG_KEY=&quot;phc_...&quot;</p>
                    </div>
                  </div>

                  {/* Internal Collector */}
                  <div className="rounded-xl border border-emerald-900/60 bg-emerald-950/20 p-5">
                    <div className="flex items-center justify-between mb-2">
                      <h3 className="font-semibold text-emerald-300">4. Internal Built-in Collector</h3>
                      <span className="rounded bg-emerald-950 px-2 py-0.5 text-[10px] font-semibold text-emerald-300 border border-emerald-800">
                        Active & Connected
                      </span>
                    </div>
                    <p className="text-xs text-slate-400 mb-4">
                      Uses modern <code className="text-cyan-400">fetchLater()</code> / <code className="text-cyan-400">sendBeacon</code> to stream batches into <code className="text-cyan-400">/api/telemetry</code> without external dependencies.
                    </p>
                    <div className="rounded bg-slate-900 p-3 font-mono text-[11px] text-slate-300 space-y-1">
                      <p>Endpoint: /api/telemetry</p>
                      <p>Batch Window: 3000ms</p>
                      <p>Max Queue: 50 events</p>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* ── TAB 5: PRIVACY & COMPLIANCE ───────────────────────────────────── */}
          {activeTab === "compliance" && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 backdrop-blur">
                <h2 className="text-lg font-bold text-white mb-2">GDPR & CCPA Compliance Architecture</h2>
                <p className="text-xs text-slate-400 mb-6">
                  Perigee guarantees zero personal data collection under the strictest privacy frameworks worldwide.
                </p>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                  <div className="space-y-4">
                    <div className="flex items-start gap-3">
                      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-900/80 text-emerald-400 text-xs">
                        ✓
                      </div>
                      <div>
                        <h4 className="font-semibold text-white text-sm">No PII Collection</h4>
                        <p className="text-xs text-slate-400 mt-0.5">
                          Stellar public keys (G...), Soroban contract IDs (C...), email addresses, IP addresses, and secret seeds are stripped or redacted before transmission.
                        </p>
                      </div>
                    </div>

                    <div className="flex items-start gap-3">
                      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-900/80 text-emerald-400 text-xs">
                        ✓
                      </div>
                      <div>
                        <h4 className="font-semibold text-white text-sm">Do Not Track (DNT) & GPC Support</h4>
                        <p className="text-xs text-slate-400 mt-0.5">
                          Automatically honors <code className="text-cyan-400">navigator.globalPrivacyControl</code> and <code className="text-cyan-400">navigator.doNotTrack</code>. Users with privacy signals enabled are opted-out by default.
                        </p>
                      </div>
                    </div>

                    <div className="flex items-start gap-3">
                      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-900/80 text-emerald-400 text-xs">
                        ✓
                      </div>
                      <div>
                        <h4 className="font-semibold text-white text-sm">30-Day Consent TTL</h4>
                        <p className="text-xs text-slate-400 mt-0.5">
                          Consent decisions automatically expire after 30 days, resurfacing the preference banner so users can review their choices.
                        </p>
                      </div>
                    </div>

                    <div className="flex items-start gap-3">
                      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-900/80 text-emerald-400 text-xs">
                        ✓
                      </div>
                      <div>
                        <h4 className="font-semibold text-white text-sm">Granular Categorization</h4>
                        <p className="text-xs text-slate-400 mt-0.5">
                          Users can independently allow or deny Performance Telemetry, Error Reporting, and Product Analytics.
                        </p>
                      </div>
                    </div>
                  </div>

                  {/* Interactive Opt-Out Simulator */}
                  <div className="rounded-xl border border-slate-800 bg-slate-950 p-5 flex flex-col justify-between">
                    <div>
                      <h4 className="font-semibold text-white text-sm mb-2">Instant Opt-Out Test</h4>
                      <p className="text-xs text-slate-400 mb-4">
                        Test the opt-out mechanism directly on this browser. When opted-out, all tracking scripts are unmounted and events are dropped immediately.
                      </p>
                      <div className="rounded-lg bg-slate-900 p-3 text-xs mb-4">
                        <span className="text-slate-400">Your Current Status: </span>
                        <span
                          className={`font-semibold ${
                            optOutStatus === true
                              ? "text-emerald-400"
                              : optOutStatus === false
                                ? "text-amber-400"
                                : "text-slate-400"
                          }`}
                        >
                          {optOutStatus === true
                            ? "Consent Granted"
                            : optOutStatus === false
                              ? "Opted Out (No Tracking)"
                              : "No Decision Yet"}
                        </span>
                      </div>
                    </div>

                    <button
                      type="button"
                      onClick={handleToggleOptOut}
                      className={`w-full rounded-lg py-2.5 text-xs font-semibold transition-colors ${
                        optOutStatus === true
                          ? "bg-red-600 hover:bg-red-500 text-white"
                          : "bg-cyan-600 hover:bg-cyan-500 text-white"
                      }`}
                    >
                      {optOutStatus === true ? "Opt Out of Telemetry" : "Opt In to Telemetry"}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      </main>
    </>
  );
}
