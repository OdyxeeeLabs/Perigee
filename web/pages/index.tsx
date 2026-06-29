import Head from 'next/head';
import { useEffect, useMemo, useState } from 'react';
import { ConnectButton } from '../components/ConnectButton';
import { ContractInteraction } from '../components/ContractInteraction';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { FunctionSidebar } from '../components/FunctionSidebar';
import { GasGolfingSuggestionsTable } from '../components/GasGolfingSuggestionsTable';
import { GasUsageChart } from '../components/GasUsageChart';
import { InvocationHistory, useInvocationHistory } from '../components/InnovocationHistory';
import { NutritionLabel } from '../components/NutritionLabel';
import { NutritionLabelSkeleton } from '../components/NutritionLabelSkeleton';
import { ResourceHeatmap } from '../components/ResourceHeatmap';
import { ResultViewer } from '../components/Resultviewer';
import { ResultViewerSkeleton } from '../components/ResultViewerSkeleton';
import { UploadZone } from '../components/upload-zone';
import { WalletModal } from '../components/WalletModal';
import { ApiError, analyzeService, apiUrl } from '../lib/api';
import { loadLatestAnalysis, saveLatestAnalysis } from '../lib/analysisStorage';
import { createUserFriendlyMessage, extractErrorDetails, formatError } from '../lib/errorHandling';
import type { GasGolfingSuggestion } from '../lib/gasGolfingSort';
import {
  generateMockResult,
  MOCK_CONTRACT_FUNCTIONS,
} from '../lib/sorobantypes';
import type { ContractFunction, InvocationResult } from '../lib/sorobantypes';

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';

  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }

  return btoa(binary);
}

export default function Home() {
  const [contractId, setContractId] = useState(
    'CAEZJVJ4N7P7GRUVD5NG5LYYH23AQHJUKQEUHW54LR5PGQX3V7FXD7Q',
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
  const [tab, setTab] = useState<'explorer' | 'history'>('explorer');
  const [wasmFile, setWasmFile] = useState<File | null>(null);
  const [wasmData, setWasmData] = useState<string | null>(null);
  const [gasGolfingSuggestions, setGasGolfingSuggestions] = useState<GasGolfingSuggestion[]>([]);
  const [gasGolfingLoading, setGasGolfingLoading] = useState(false);
  const [gasGolfingError, setGasGolfingError] = useState<string | null>(null);
  const { history, addToHistory } = useInvocationHistory();

  useEffect(() => {
    const restored = loadLatestAnalysis();
    if (restored) {
      setCurrentResult(restored);
    }
  }, []);

  const analysisReport = useMemo(
    () => currentResult?.analysisReport ?? currentResult?.resourceCost ?? null,
    [currentResult],
  );

  const storeResult = (result: InvocationResult) => {
    setCurrentResult(result);
    addToHistory(result);
    saveLatestAnalysis(result);
  };

  const handleSimulate = async (inputs: Record<string, any>, customWasmData?: string) => {
    setLoading(true);
    const activeWasmData = customWasmData || wasmData;

    try {
      const args = Object.values(inputs).map((value) => String(value));
  const [wasmData, setWasmData] = useState<string | null>(null);

  useEffect(() => {
    setCurrentResult(null);
  }, []);

  const handleSimulate = async (inputs: Record<string, any>, customWasmData?: string) => {
    setLoading(true);
    try {
      const activeWasmData = customWasmData ?? wasmData;
      const report = activeWasmData
        ? await analyzeService.analyzeWasm({
            wasm_bytes: activeWasmData,
            function_name: selectedFunction.name,
            args,
            args: Object.values(inputs).map((value) => String(value)),
          })
        : await analyzeService.analyze({
            contract_id: contractId,
            function_name: selectedFunction.name,
            args,
          });

      storeResult({
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
      });
    } catch (error) {
      const formatted = formatError(error);
      const apiErrorType = error instanceof ApiError ? error.body?.error : undefined;

      storeResult({
        id: Math.random().toString(36).slice(2),
        functionName: selectedFunction.name,
        inputs,
        error: formatted.message,
        errorType: apiErrorType || formatted.type,
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

  const handleGasGolfing = async (file: File) => {
    setGasGolfingLoading(true);
    setGasGolfingError(null);
    setGasGolfingSuggestions([]);

    try {
      const wasmBytes = arrayBufferToBase64(await file.arrayBuffer());
      const response = await fetch(apiUrl('/analyze/gas-golfing'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          wasm_bytes: wasmBytes,
          contract_name: file.name.replace(/\.wasm$/i, ''),
        }),
      });

      if (!response.ok) {
        const errorResponse = await extractErrorDetails(response);
        throw new Error(createUserFriendlyMessage(errorResponse));
      }

      const data = await response.json();
      setGasGolfingSuggestions((data?.report?.suggestions ?? []) as GasGolfingSuggestion[]);
    } catch (error) {
      setGasGolfingError(error instanceof Error ? error.message : 'Failed to analyze WASM');
    } finally {
      setGasGolfingLoading(false);
    }
  };

  const handleFileReady = async (file: File) => {
    setWasmFile(file);
    const base64 = arrayBufferToBase64(await file.arrayBuffer());
    setWasmData(base64);
    await Promise.all([handleSimulate({}, base64), handleGasGolfing(file)]);
  };

  const clearAnalysis = () => {
    setWasmFile(null);
    setWasmData(null);
    setCurrentResult(null);
    setGasGolfingSuggestions([]);
    setGasGolfingError(null);
  };

  return (
    <>
      <Head>
        <title>SoroScope - Soroban Smart Contract Resource Analyzer</title>
        <meta
          name="description"
          content="Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts."
        />
        <meta name="viewport" content="width=device-width, initial-scale=1.0" />
        <link rel="icon" href="/favicon.ico" />
      </Head>

      <div style={{ minHeight: '100vh', backgroundColor: '#0f1117' }}>
        <header className="sticky top-0 z-[100] flex flex-col gap-4 border-b border-[#30363d] bg-[#1a1f26] px-6 py-6 sm:flex-row sm:items-center sm:justify-between sm:px-10 lg:pl-[140px] lg:pr-[125px]">
          <div className="max-w-[1200px]">
            <h1 style={{ margin: '0 0 12px 0', fontSize: '28px', fontWeight: '700', color: '#00d9ff' }}>
              SoroScope
            </h1>
            <p style={{ margin: 0, color: '#8b949e', fontSize: '14px' }}>
              Explore and test Soroban smart contracts with precision
            </p>
          </div>
          <ConnectButton />
        </header>

        <main className="mx-auto max-w-[1200px] px-4 py-6 sm:px-6">
          <section className="mb-6 rounded-xl border border-[#30363d] bg-[#161b22] p-7">
            <div className="mb-4">
              <h2 className="m-0 text-base font-semibold text-[#c9d1d9]">Upload Contract</h2>
              <p className="m-0 text-[13px] text-[#8b949e]">
                Drop a compiled Soroban contract (.wasm) to analyze its resource usage
              </p>
            </div>

            <ErrorBoundary
              fallback={(error, reset) => (
                <div className="rounded-lg border border-red-800/60 bg-red-950/30 p-6 text-center text-red-100">
                  <p className="text-sm font-semibold">Upload failed unexpectedly</p>
                  <p className="mx-auto mt-2 max-w-md text-xs leading-relaxed text-red-200/80">
                    {error.message}
                  </p>
                  <button
                    type="button"
                    onClick={reset}
                    className="mt-4 rounded-md border border-red-700/70 px-4 py-2 text-sm text-red-100 hover:bg-red-900/40"
                  >
                    Try another file
                  </button>
                </div>
              )}
            >
              <UploadZone
                backendUrl={apiUrl('/analyze/wasm')}
                enableBackendValidation
                onFileReady={(file) => {
                  void handleFileReady(file);
                }}
                onReset={clearAnalysis}
              />
            </ErrorBoundary>
          </section>

          <section className="mb-6">
            {gasGolfingLoading ? (
              <div className="rounded-lg border border-[#30363d] bg-[#0d1117] p-4 text-sm text-[#8b949e]">
                Analyzing WASM for Gas Golfing suggestions...
              </div>
            ) : gasGolfingError ? (
              <div className="rounded-lg border border-[#fb8500] bg-[#0d1117] p-4 text-sm text-[#f0883e]">
                {gasGolfingError}
              </div>
            ) : gasGolfingSuggestions.length > 0 ? (
              <GasGolfingSuggestionsTable suggestions={gasGolfingSuggestions} />
            ) : null}
          </section>

          <section className="mb-6 rounded-lg border border-[#30363d] bg-[#161b22] p-6">
            <label className="mb-2 block font-medium text-[#c9d1d9]">Contract ID</label>
            <input
              type="text"
              value={contractId}
              onChange={(event) => setContractId(event.target.value)}
              placeholder="Enter Soroban contract ID"
              className="w-full rounded-md border border-[#30363d] bg-[#0d1117] px-4 py-3 font-mono text-sm text-[#c9d1d9]"
            />
            <p className="mt-2 text-xs text-[#8b949e]">
              Contract ID: <code className="text-[#00d9ff]">{contractId.substring(0, 20)}...</code>
            </p>
            {wasmFile && (
              <div className="mt-4 flex items-center gap-2 rounded-md border border-emerald-400/25 bg-emerald-400/10 p-3">
                <span className="text-xs font-semibold text-emerald-400">Active WASM:</span>
                <code className="font-mono text-xs text-[#c9d1d9]">{wasmFile.name}</code>
                <span className="text-[11px] text-[#8b949e]">({(wasmFile.size / 1024).toFixed(1)} KB)</span>
              </div>
            )}
          </section>

          <div className="mb-6 grid grid-cols-1 gap-6 lg:grid-cols-2">
            <div>
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
              <ContractInteraction selectedFunction={selectedFunction} loading={loading} onSubmit={handleSimulate} />
            </div>

            <div>
              <div className="mb-6 flex rounded-t-lg border-b border-[#30363d] bg-[#161b22]">
                <button
                  type="button"
                  onClick={() => setTab('explorer')}
                  className={`flex-1 px-4 py-3 text-sm ${
                    tab === 'explorer' ? 'border-b-2 border-[#00d9ff] font-semibold text-[#00d9ff]' : 'text-[#8b949e]'
                  }`}
                >
                  Result
                </button>
                <button
                  type="button"
                  onClick={() => setTab('history')}
                  className={`flex-1 px-4 py-3 text-sm ${
                    tab === 'history' ? 'border-b-2 border-[#00d9ff] font-semibold text-[#00d9ff]' : 'text-[#8b949e]'
                  }`}
                >
                  History ({history.length})
                </button>
              </div>

              <div className="rounded-b-lg border border-t-0 border-[#30363d] bg-[#161b22] p-6">
                {tab === 'explorer' ? (
                  loading ? (
                    <>
                      <ResultViewerSkeleton />
                      <div className="mt-4">
                        <NutritionLabelSkeleton />
                      </div>
                    </>
                  ) : (
                    <>
                      <ResultViewer result={currentResult} />
                      {analysisReport && (
                        <div className="mt-4 flex flex-col gap-4">
                          <ResourceHeatmap
                            resourceCost={{
                              cpu_instructions: analysisReport.cpu_instructions,
                              ram_bytes: analysisReport.ram_bytes,
                              ledger_read_bytes: analysisReport.ledger_read_bytes,
                              ledger_write_bytes: analysisReport.ledger_write_bytes,
                              transaction_size_bytes: analysisReport.transaction_size_bytes,
                              cost_stroops: analysisReport.cost_stroops,
                              state_snapshot: currentResult?.stateSnapshot,
                            }}
                          />
                          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
                            <NutritionLabel
                              cpu_instructions={analysisReport.cpu_instructions}
                              ram_bytes={analysisReport.ram_bytes}
                              ledger_read_bytes={analysisReport.ledger_read_bytes}
                              ledger_write_bytes={analysisReport.ledger_write_bytes}
                              transaction_size_bytes={analysisReport.transaction_size_bytes}
                            />
                            <GasUsageChart
                              cpu_instructions={analysisReport.cpu_instructions}
                              ram_bytes={analysisReport.ram_bytes}
                              ledger_read_bytes={analysisReport.ledger_read_bytes}
                              ledger_write_bytes={analysisReport.ledger_write_bytes}
                              transaction_size_bytes={analysisReport.transaction_size_bytes}
                              cost_stroops={analysisReport.cost_stroops}
                              testnetAverages={analysisReport.testnet_averages}
                            />
                          </div>
                          <button
                            type="button"
                            onClick={clearAnalysis}
                            className="self-start rounded bg-slate-800 px-4 py-2 text-sm text-slate-300 transition hover:bg-slate-700"
                          >
                            Clear analysis
                          </button>
                        </div>
                      )}
                    </>
                  )
                ) : (
                  <InvocationHistory
                    onSelectResult={(result) => {
                      setCurrentResult(result);
                      setTab('explorer');
                    }}
                  />
                )}
              </div>
            </div>
          </div>

          <section className="grid gap-4 sm:grid-cols-3">
            {[
              ['Simulate', 'Preview contract execution without signing or spending XLM', '#00d9ff'],
              ['Invoke', 'Execute real transactions via your connected wallet', '#a371f7'],
              ['History', 'Track all function calls with full details and resource costs', '#fb8500'],
            ].map(([title, body, color]) => (
              <div key={title} className="rounded-lg border border-[#30363d] bg-[#161b22] p-4" style={{ borderLeft: `4px solid ${color}` }}>
                <h3 className="mb-2 text-sm font-semibold" style={{ color }}>
                  {title}
                </h3>
                <p className="m-0 text-[13px] text-[#8b949e]">{body}</p>
              </div>
            ))}
          </section>
        </main>

        <WalletModal />
      </div>
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

            <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
              <ResultViewer result={currentResult} />
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
