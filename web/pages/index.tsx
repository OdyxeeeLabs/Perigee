import Head from "next/head";
import { useEffect, useState } from "react";

import { ConnectButton } from "../components/ConnectButton";
import { ContractInteraction } from "../components/ContractInteraction";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { FunctionSidebar } from "../components/FunctionSidebar";
import { ResultViewer } from "../components/Resultviewer";
import { UploadZone } from "../components/upload-zone";
import { analyzeService } from "../lib/api";
import {
  MOCK_CONTRACT_FUNCTIONS,
  generateMockResult,
  type ContractFunction,
  type InvocationResult,
} from "../lib/sorobantypes";

export default function Home() {
  const [contractId, setContractId] = useState(
    "CAEZJVJ4N7P7GRUVD5NG5LYYH23AQHJUKQEUHW54LR5PGQX3V7FXD7Q",
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

  const handleSimulate = async (inputs: Record<string, any>, customWasmData?: string) => {
    setLoading(true);
    try {
      const url = activeWasmData ? 'http://localhost:8080/analyze/wasm' : 'http://localhost:8080/analyze';
      const body = activeWasmData
        ? {
      const activeWasmData = customWasmData ?? wasmData;
      const report = activeWasmData
        ? await analyzeService.analyzeWasm({
            wasm_bytes: activeWasmData,
            function_name: selectedFunction.name,
            args: Object.values(inputs).map((value) => String(value)),
          })
        : await analyzeService.analyze({
            contract_id: contractId,
            function_name: selectedFunction.name,
          });

      const result: InvocationResult = {
        id: Math.random().toString(36).slice(2),
        functionName: selectedFunction.name,
        inputs,
        result: generateMockResult(selectedFunction.name, inputs),
        analysisReport: report,
        resourceCost: report,
        stateSnapshot: report.state_snapshot,
        callGraphMermaid: report.call_graph_mermaid,
        timestamp: Date.now(),
        success: true,
      };

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
  };

  return (
    <>
      <Head>
        <title>SoroScope - Soroban Smart Contract Resource Analyzer</title>
        <meta
          name="description"
          content="Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts."
        />
      </Head>
      <main className="min-h-screen bg-slate-950 text-slate-100">
        <header className="sticky top-0 z-50 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
            <div>
              <h1 className="text-2xl font-bold text-cyan-400">SoroScope</h1>
              <p className="text-sm text-slate-400">Soroban analysis workspace</p>
            </div>
            <ConnectButton />
          </div>

          {/* Right Column - Results & History Tabs */}
          <div>
            {/* Tabs */}
            <div
              style={{
                display: 'flex',
                borderBottom: '1px solid #30363d',
                marginBottom: '24px',
                backgroundColor: '#161b22',
                borderRadius: '8px 8px 0 0',
                gap: '0',
              }}
            >
              <button
                onClick={() => setTab('explorer')}
                style={{
                  flex: 1,
                  padding: '12px 16px',
                  backgroundColor: 'transparent',
                  border: 'none',
                  borderBottom: tab === 'explorer' ? '2px solid #00d9ff' : 'none',
                  cursor: 'pointer',
                  fontSize: '14px',
                  fontWeight: tab === 'explorer' ? '600' : '500',
                  color: tab === 'explorer' ? '#00d9ff' : '#8b949e',
                }}
              >
                Result
              </button>
              <button
                onClick={() => setTab('history')}
                style={{
                  flex: 1,
                  padding: '12px 16px',
                  backgroundColor: 'transparent',
                  border: 'none',
                  borderBottom: tab === 'history' ? '2px solid #00d9ff' : 'none',
                  cursor: 'pointer',
                  fontSize: '14px',
                  fontWeight: tab === 'history' ? '600' : '500',
                  color: tab === 'history' ? '#00d9ff' : '#8b949e',
        </header>

        <section className="mx-auto max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
          <div className="mb-6 rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
            <ErrorBoundary fallback={() => <div>Upload failed</div>}>
              <UploadZone
                onFileReady={(file) => {
                  void file;
                  setWasmData(null);
                }}
              />
            </ErrorBoundary>
          </div>

          <div className="grid gap-6 lg:grid-cols-2">
            <div className="space-y-4">
              <FunctionSidebar
                functions={MOCK_CONTRACT_FUNCTIONS}
                selectedFunction={selectedFunction}
                onSelect={(func) => {
                  setSelectedFunction(func);
                  setCurrentResult(null);
                }}
              />
              <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
                <label className="mb-2 block text-sm font-medium text-slate-300">
                  Contract ID
                </label>
                <input
                  value={contractId}
                  onChange={(e) => setContractId(e.target.value)}
                  className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-100"
                />
              </div>
              <ContractInteraction
                selectedFunction={selectedFunction}
                loading={loading}
                onSubmit={(inputs) => void handleSimulate(inputs)}
              />
            </div>

            {/* Tab Content */}
            <div
              style={{
                backgroundColor: '#161b22',
                borderRadius: '0 8px 8px 8px',
                padding: '24px',
                border: '1px solid #30363d',
                borderTop: 'none',
              }}
            >
              {tab === 'explorer' ? (
                loading ? (
                  <>
                    <ResultViewerSkeleton />
                    <div className="mt-4">
                      <NutritionLabelSkeleton />
                <>
                  <ResultViewer result={currentResult} />
                  {currentResult?.resourceCost && (
                    <div className="mt-4 flex flex-col gap-4">
                      <ResourceHeatmap resourceCost={{
                        cpu_instructions: currentResult.resourceCost.cpu_instructions,
                        ram_bytes: currentResult.resourceCost.ram_bytes,
                        ledger_read_bytes: currentResult.resourceCost.ledger_read_bytes,
                        ledger_write_bytes: currentResult.resourceCost.ledger_write_bytes,
                        transaction_size_bytes: currentResult.resourceCost.transaction_size_bytes,
                        cost_stroops: (currentResult.resourceCost as any).cost_stroops,
                        state_snapshot: currentResult.stateSnapshot
                      }} />
                  {analysisReport && (
                    <button
                      type="button"
                      onClick={() => {
                        setCurrentResult(null);
                        const resetBtn = document.getElementById('wasm-upload-reset-btn');
                        if (resetBtn) resetBtn.click();
                      }}
                      className="mt-4 px-4 py-2 bg-slate-800 text-slate-300 rounded hover:bg-slate-700 transition"
                    >
                      Clear analysis
                    </button>
                  )}
                  {currentResult?.resourceCost && (
                    <div className="mt-4">
                    <div className="mt-4 grid grid-cols-1 lg:grid-cols-2 gap-4">
                      <NutritionLabel
                        cpu_instructions={analysisReport.cpu_instructions}
                        ram_bytes={analysisReport.ram_bytes}
                        ledger_read_bytes={analysisReport.ledger_read_bytes}
                        ledger_write_bytes={analysisReport.ledger_write_bytes}
                        transaction_size_bytes={analysisReport.transaction_size_bytes}
                      />
                      <GasUsageChart
                        cpu_instructions={currentResult.resourceCost.cpu_instructions}
                        ram_bytes={currentResult.resourceCost.ram_bytes}
                        ledger_read_bytes={currentResult.resourceCost.ledger_read_bytes}
                        ledger_write_bytes={currentResult.resourceCost.ledger_write_bytes}
                        transaction_size_bytes={currentResult.resourceCost.transaction_size_bytes}
                        cost_stroops={currentResult.resourceCost.cost_stroops}
                        testnetAverages={currentResult.resourceCost.testnet_averages}
                      />
                    </div>
                  </>
                ) : (
                  <>
                    <ResultViewer result={currentResult} />
                    {currentResult?.resourceCost && (
                      <div className="mt-4">
                        <NutritionLabel
                          cpu_instructions={currentResult.resourceCost.cpu_instructions}
                          ram_bytes={currentResult.resourceCost.ram_bytes}
                          ledger_read_bytes={currentResult.resourceCost.ledger_read_bytes}
                          ledger_write_bytes={currentResult.resourceCost.ledger_write_bytes}
                          transaction_size_bytes={currentResult.resourceCost.transaction_size_bytes}
                        />
                      </div>
                    )}
                  </>
                )
              ) : (
                <InvocationHistory onSelectResult={(result) => {
                  setCurrentResult(result);
                  setTab('explorer');
                }} />
              )}
            <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
              <ResultViewer result={currentResult} />
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
