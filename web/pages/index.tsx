import Head from "next/head";
import { useEffect, useState, useCallback, useMemo } from "react";
import { useTranslations } from "next-intl";

import { ConnectButton } from "../components/ConnectButton";
import { ContractInteraction } from "../components/ContractInteraction";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { FunctionSidebar } from "../components/FunctionSidebar";
import { ResultViewer } from "../components/Resultviewer";
import { SEO } from "../components/SEO";
import { UploadZone } from "../components/upload-zone";
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
            {currentResult && (
              <ErrorBoundary>
                <ResultViewer result={currentResult} />
              </ErrorBoundary>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}
