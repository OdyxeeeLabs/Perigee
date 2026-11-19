import Head from "next/head";
import dynamic from "next/dynamic";
import { useEffect, useState, useCallback, useMemo } from "react";
import { useTranslations } from "next-intl";

import { ConnectButton } from "../components/ConnectButton";
import { ContractInteraction } from "../components/ContractInteraction";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { FunctionSidebar } from "../components/FunctionSidebar";
import { LiveAlert, LiveStatus } from "../components/LiveRegion";
import { SEO } from "../components/SEO";
import { UploadZone } from "../components/upload-zone";
import { ResultViewerSkeleton } from "../components/ResultViewerSkeleton";

/**
 * Lazy-load ResultViewer — it pulls in `mermaid` (~1.5 MB) which is only
 * needed after the user has triggered an analysis. Loading it dynamically
 * keeps it out of the initial JS bundle.
 *
 * Resolves #686 — Optimize bundle size.
 *
 * The chunk-loading fallback below used to be a plain "Loading…" string
 * (#685: spinner-less, non-content-aware, and causes layout shift once the
 * real panel mounts). `ResultViewerSkeleton` already existed in the
 * component library but was unused anywhere in the app — this is its first
 * real usage, matching the exact dimensions of the panel it is standing in
 * for.
 */
const ResultViewer = dynamic(
  () => import("../components/Resultviewer").then((m) => ({ default: m.ResultViewer })),
  {
    ssr: false,
    loading: () => <ResultViewerSkeleton />,
  },
);
import { analyzeService } from "../lib/api";
import { trackTelemetryEvent } from "../lib/telemetry";
import { contractIds } from "../lib/contracts.config";
import { sanitizeUserInput } from "../lib/sanitize";
import {
  MOCK_CONTRACT_FUNCTIONS,
  generateMockResult,
  type ContractFunction,
  type InvocationResult,
  type SimulationInputs,
} from "../lib/sorobantypes";

export default function Home() {
  const t = useTranslations();
  const [contractId, setContractId] = useState(
    contractIds.helloSoroban ?? "",
  );
  const [selectedFunction, setSelectedFunction] = useState<ContractFunction>(
    MOCK_CONTRACT_FUNCTIONS[0],
  );
  const [currentResult, setCurrentResult] = useState<InvocationResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [wasmData, setWasmData] = useState<string | null>(null);

  // WEB-16 (#470): loading and outcome changes are announced to screen
  // readers through live regions (see components/LiveRegion.tsx). Derived
  // from state, so the announcement always matches what is on screen.
  const analysisStatusMessage = loading
    ? t("a11y.analysisRunning")
    : currentResult
      ? t("a11y.resultReady", {
          outcome: currentResult.success ? t("result.success") : t("result.error"),
        })
      : "";

  useEffect(() => {
    setCurrentResult(null);
  }, []);

  // Memoize the simulate handler so child components that receive it as a prop
  // (ContractInteraction, FunctionSidebar) do not re-render on every keystroke
  // in unrelated input fields. Resolves WEB-28 (#114).
  const handleSimulate = useCallback(
    async (inputs: SimulationInputs, customWasmData?: string) => {
      setLoading(true);
      const activeWasmData = customWasmData ?? wasmData;
      const sanitizedInputs = Object.fromEntries(
        Object.entries(inputs).map(([key, value]) => [
          key,
          typeof value === "string" ? sanitizeUserInput(value, key) : value,
        ]),
      ) as SimulationInputs;

      try {
        const report = activeWasmData
          ? await analyzeService.analyzeWasm({
              wasm_bytes: activeWasmData,
              function_name: selectedFunction.name,
              args: Object.values(sanitizedInputs).map((value) => String(value)),
            })
          : await analyzeService.analyze({
              contract_id: contractId,
              function_name: selectedFunction.name,
              args: Object.values(sanitizedInputs).map((value) => String(value)),
            });

        const result: InvocationResult = {
          id: Math.random().toString(36).slice(2),
          functionName: selectedFunction.name,
          inputs,
          result: generateMockResult(selectedFunction.name, inputs),
          analysisReport: report,
          resourceCost: report,
          stateSnapshot: report.state_snapshot ?? undefined,
          callGraphMermaid: report.call_graph_mermaid ?? undefined,
          timestamp: Date.now(),
          success: true,
        };

        trackTelemetryEvent({
          name: "contract_analyze",
          properties: {
            mode: activeWasmData ? "wasm" : "contract_id",
            fn: selectedFunction.name,
          },
        });

        setCurrentResult(result);
      } catch (error) {
        const message = error instanceof Error ? error.message : "Analysis failed";
        setCurrentResult({
          id: Math.random().toString(36).slice(2),
          functionName: selectedFunction.name,
          inputs,
          error: message,
          errorType: "ANALYSIS_ERROR",
          timestamp: Date.now(),
          success: false,
        });
      } finally {
        setLoading(false);
      }
    },
    // Re-create only when the selected function, contract ID or wasm data change —
    // not on every parent render.
    [contractId, selectedFunction, wasmData],
  );

  // Memoize the function-selection handler for the same reason.
  const handleSelectFunction = useCallback((fn: ContractFunction) => {
    setSelectedFunction(fn);
  }, []);

  // Derive the wasm-upload handler once; it only depends on stable setters.
  const handleWasmUpload = useCallback((data: string) => {
    setWasmData(data);
  }, []);

  // Compute the sidebar function list once — MOCK_CONTRACT_FUNCTIONS is a
  // module-level constant so this memo is effectively free after first render.
  const functionList = useMemo(() => MOCK_CONTRACT_FUNCTIONS, []);

  return (
    <ErrorBoundary
      variant="route"
      context={{
        route: "/",
        component: "HomePage",
        props: {
          contractId,
          selectedFunction: selectedFunction.name,
          hasResult: Boolean(currentResult),
        },
      }}
      title="This dashboard failed to load"
      description="The dashboard could not render correctly. Try again to recover the page state."
    >
      <main className="flex min-h-screen flex-col items-center bg-slate-950 text-slate-100">
        <Head>
          <link rel="icon" href="/favicon.ico" />
        </Head>
        <SEO
          title={t("home.heading")}
          description={t("home.description")}
          path="/"
        />

        <div className="w-full max-w-7xl px-4 py-8">
          <header className="mb-8 flex items-center justify-between">
            <h1 className="text-2xl font-bold text-sky-400">{t("home.title")}</h1>
            <ConnectButton />
          </header>

          <div className="grid grid-cols-1 gap-6 lg:grid-cols-[280px_1fr]">
            <aside>
              <FunctionSidebar
                functions={functionList}
                selected={selectedFunction}
                onSelect={handleSelectFunction}
              />
            </aside>

            <div className="flex flex-col gap-6">
              <UploadZone onUpload={handleWasmUpload} />
              <ContractInteraction
                contractId={contractId}
                onContractIdChange={setContractId}
                selectedFunction={selectedFunction}
                onSimulate={handleSimulate}
                isLoading={loading}
              />
              {/* WEB-16 (#470): announce analysis progress and outcomes. */}
              <LiveStatus
                message={analysisStatusMessage}
                label={t("a11y.statusRegionLabel")}
                data-testid="analysis-live-status"
              />
              <LiveAlert
                message={
                  currentResult && !currentResult.success
                    ? (currentResult.error ?? t("a11y.analysisFailed"))
                    : ""
                }
                data-testid="analysis-live-alert"
              />
              {/*
               * #685: while a simulate/invoke is in flight, show the same
               * content-aware skeleton used for the lazy chunk load above
               * instead of leaving stale results on screen or nothing at
               * all — the panel's final size is already known, so this also
               * avoids layout shift once the real result replaces it.
               */}
              {loading && <ResultViewerSkeleton />}
              {!loading && currentResult && (
                <ErrorBoundary
                  variant="component"
                  context={{
                    route: "/",
                    component: "ResultViewer",
                    props: {
                      functionName: currentResult.functionName,
                      success: currentResult.success,
                    },
                  }}
                  title="The result panel failed to load"
                  description="The analysis result could not be displayed, but the rest of the dashboard remains available."
                >
                  <ResultViewer result={currentResult} />
                </ErrorBoundary>
              )}
            </div>
          </div>
        </div>
      </main>
    </ErrorBoundary>
  );
}
