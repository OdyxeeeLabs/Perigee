import Head from 'next/head';
import { useState, useEffect } from 'react';
import { ResultViewer } from '../components/Resultviewer';
import { InvocationHistory, useInvocationHistory } from '../components/InnovocationHistory';
import { NutritionLabel } from '../components/NutritionLabel';
import { FunctionSidebar } from '../components/FunctionSidebar';
import { ContractInteraction } from '../components/ContractInteraction';
import { MOCK_CONTRACT_FUNCTIONS, generateMockResult } from '../lib/sorobantypes';
import type { ContractFunction, InvocationResult } from '../lib/sorobantypes';
import { GasUsageChart } from '../components/GasUsageChart';
import { UploadZone } from '../components/upload-zone';
import { extractErrorDetails, createUserFriendlyMessage, formatError } from '../lib/errorHandling';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { ResultViewerSkeleton } from '../components/ResultViewerSkeleton';
import { NutritionLabelSkeleton } from '../components/NutritionLabelSkeleton';
import { VaultBalanceSkeleton } from '../components/VaultBalanceSkeleton';
import { StrategyPhaseSkeleton } from '../components/StrategyPhaseSkeleton';
import { NAVSkeleton } from '../components/NAVSkeleton';
import { ApiError } from '../lib/api';
import { ResourceHeatmap } from '../components/ResourceHeatmap';
import { GasGolfingSuggestionsTable } from '../components/GasGolfingSuggestionsTable';
import { Skeleton } from '../components/Skeleton';
import type { GasGolfingSuggestion } from '../lib/gasGolfingSort';
import { apiUrl } from '../lib/api';
import { saveLatestAnalysis, loadLatestAnalysis } from '../lib/analysisStorage';
import { ConnectButton } from '../components/ConnectButton';
import { WalletModal } from '../components/WalletModal';

// ─── Helper ──────────────────────────────────────────────────────────────────

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

// ─── Component ───────────────────────────────────────────────────────────────

export default function Home() {
  const [contractId, setContractId] = useState(
    'CAEZJVJ4N7P7GRUVD5NG5LYYH23AQHJUKQEUHW54LR5PGQX3V7FXD7Q',
  );
  const [selectedFunction, setSelectedFunction] = useState<ContractFunction>(
    MOCK_CONTRACT_FUNCTIONS[0],
  );
  const [currentResult, setCurrentResult] = useState<InvocationResult | null>(null);

  // Per-section loading states so every data region gets its own skeleton
  const [loading, setLoading] = useState(false);
  const [gasGolfingLoading, setGasGolfingLoading] = useState(false);
  const [gasGolfingError, setGasGolfingError] = useState<string | null>(null);
  const [gasGolfingSuggestions, setGasGolfingSuggestions] = useState<GasGolfingSuggestion[]>([]);

  /**
   * dashboardLoading tracks the initial backend round-trip that populates the
   * vault balance, strategy phase, and NAV panels.  It starts true and resolves
   * to false once that data lands (or on error).  In this implementation the
   * panels use the analysis result as their data source; once any analysis has
   * finished or been restored from localStorage we stop showing skeletons.
   */
  const [dashboardLoading, setDashboardLoading] = useState(true);

  const [tab, setTab] = useState<'explorer' | 'history'>('explorer');
  const { history, addToHistory } = useInvocationHistory();
  const [wasmFile, setWasmFile] = useState<File | null>(null);
  const [wasmData, setWasmData] = useState<string | null>(null);

  // Restore the latest analysis result on initial page load
  useEffect(() => {
    const restored = loadLatestAnalysis();
    if (restored) {
      setCurrentResult(restored);
    }
    // Once we've attempted a restore we're done with the initial dashboard load
    setDashboardLoading(false);
  }, []);

  // ─── Handlers ──────────────────────────────────────────────────────────────

  const handleSimulate = async (inputs: Record<string, unknown>, customWasmData?: string) => {
    setLoading(true);
    setDashboardLoading(true);
    let errorType: string | undefined;
    const activeWasmData = customWasmData || wasmData;

    try {
      const url = activeWasmData ? apiUrl('/analyze/wasm') : apiUrl('/analyze');
      const body = activeWasmData
        ? {
            wasm_bytes: activeWasmData,
            function_name: selectedFunction.name,
            args: Object.values(inputs).map((val) => String(val)),
          }
        : {
            contract_id: contractId,
            function_name: selectedFunction.name,
          };

      const response = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });

      if (!response.ok) {
        throw new Error(`Backend error: ${response.statusText}`);
      }

      const report = await response.json();

      const result: InvocationResult = {
        id: Math.random().toString(36).substring(7),
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
      addToHistory(result);
      saveLatestAnalysis(result);
    } catch (error) {
      if (error instanceof ApiError) {
        errorType = error.body?.error;
      }

      const formatted = formatError(error);

      const errorResult: InvocationResult = {
        id: Math.random().toString(36).substring(7),
        functionName: selectedFunction.name,
        inputs,
        error: formatted.message,
        errorType: errorType || formatted.type,
        timestamp: Date.now(),
        success: false,
      };
      setCurrentResult(errorResult);
      addToHistory(errorResult);
      saveLatestAnalysis(errorResult);
    } finally {
      setLoading(false);
      setDashboardLoading(false);
    }
  };

  const handleFileAnalysis = async (file: File) => {
    setLoading(true);
    setDashboardLoading(true);
    let errorType: string | undefined;

    try {
      const arrayBuffer = await file.arrayBuffer();
      const response = await fetch(apiUrl('/analyze'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: arrayBuffer,
      });

      if (!response.ok) {
        const errorResponse = await extractErrorDetails(response);
        errorType = errorResponse.error;
        const userMessage = createUserFriendlyMessage(errorResponse);
        throw new Error(userMessage);
      }

      const report = await response.json();

      const result: InvocationResult = {
        id: Math.random().toString(36).substring(7),
        functionName: 'WASM Analysis',
        inputs: {},
        result: null,
        resourceCost: report,
        stateSnapshot: report.state_snapshot,
        callGraphMermaid: report.call_graph_mermaid,
        timestamp: Date.now(),
        success: true,
      };

      setCurrentResult(result);
      addToHistory(result);
      saveLatestAnalysis(result);
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : 'An unexpected error occurred during analysis';

      const errorResult: InvocationResult = {
        id: Math.random().toString(36).substring(7),
        functionName: 'WASM Analysis',
        inputs: {},
        error: errorMessage,
        errorType: errorType || 'UNKNOWN_ERROR',
        timestamp: Date.now(),
        success: false,
      };
      setCurrentResult(errorResult);
      addToHistory(errorResult);
      saveLatestAnalysis(errorResult);
    } finally {
      setLoading(false);
      setDashboardLoading(false);
    }
  };

  const handleWasmReady = async (file: File) => {
    setGasGolfingLoading(true);
    setGasGolfingError(null);
    setGasGolfingSuggestions([]);

    try {
      const bytes = await file.arrayBuffer();
      const wasmBytes = arrayBufferToBase64(bytes);
      const res = await fetch(apiUrl('/analyze/gas-golfing'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          wasm_bytes: wasmBytes,
          contract_name: file.name.replace(/\.wasm$/i, ''),
        }),
      });

      if (!res.ok) {
        const err = await extractErrorDetails(res);
        throw new Error(createUserFriendlyMessage(err));
      }

      const data = await res.json();
      setGasGolfingSuggestions(
        (data?.report?.suggestions ?? []) as GasGolfingSuggestion[],
      );
    } catch (e) {
      setGasGolfingError(e instanceof Error ? e.message : 'Failed to analyze WASM');
    } finally {
      setGasGolfingLoading(false);
    }
  };

  const analysisReport = currentResult?.analysisReport ?? currentResult?.resourceCost;

  // ─── Render ────────────────────────────────────────────────────────────────

  return (
    <>
      <Head>
        <title>Perigee - Soroban Smart Contract Resource Analyzer</title>
        <meta
          name="description"
          content="Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts with absolute precision, utilizing live node queries and direct WASM bytecode analysis."
        />
        <meta name="viewport" content="width=device-width, initial-scale=1.0" />
        <link rel="icon" href="/favicon.ico" />
      </Head>

      <div style={{ minHeight: '100vh', backgroundColor: '#0f1117' }}>
        {/* ── Header ─────────────────────────────────────────────────────── */}
        <header className="sticky top-0 z-[100] flex flex-col gap-4 border-b border-[#30363d] bg-[#1a1f26] px-6 py-6 sm:flex-row sm:items-center sm:justify-between sm:px-10 lg:pl-[140px] lg:pr-[125px]">
          <div className="max-w-[1200px]">
            <h1
              style={{
                margin: '0 0 12px 0',
                fontSize: '28px',
                fontWeight: '700',
                color: '#00d9ff',
                letterSpacing: '0.5px',
              }}
            >
              Perigee
            </h1>
            <p style={{ margin: '0', color: '#8b949e', fontSize: '14px' }}>
              Explore and test Soroban smart contracts with precision
            </p>
          </div>
          <div>
            <ConnectButton />
          </div>
        </header>

        {/* ── Main ───────────────────────────────────────────────────────── */}
        <main className="mx-auto max-w-[1200px] px-4 py-6 sm:px-6">

          {/* ── Dashboard panels: Vault Balance / Strategy Phase / NAV ───── */}
          {/*
            These three panels represent the live on-chain data for a user's
            vault. While dashboardLoading is true we render skeleton placeholders
            so the layout is stable and there is no flash of empty content.
          */}
          <div className="mb-6 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {/* Vault Balance */}
            {dashboardLoading ? (
              <VaultBalanceSkeleton />
            ) : (
              <div className="rounded-xl border border-[#30363d] bg-[#161b22] p-6">
                <div className="mb-4 flex items-center justify-between">
                  <h2
                    style={{
                      margin: 0,
                      fontSize: '14px',
                      fontWeight: '600',
                      color: '#c9d1d9',
                    }}
                  >
                    Vault Balance
                  </h2>
                  <span
                    style={{
                      padding: '2px 10px',
                      borderRadius: '9999px',
                      fontSize: '11px',
                      fontWeight: '600',
                      backgroundColor: 'rgba(0, 217, 255, 0.12)',
                      color: '#00d9ff',
                      border: '1px solid rgba(0, 217, 255, 0.25)',
                    }}
                  >
                    Live
                  </span>
                </div>
                <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
                  {currentResult
                    ? 'Vault data refreshed after last analysis.'
                    : 'Upload a contract or run a simulation to populate vault metrics.'}
                </p>
              </div>
            )}

            {/* Strategy Phase */}
            {dashboardLoading ? (
              <StrategyPhaseSkeleton />
            ) : (
              <div
                className="rounded-xl border border-[#30363d] bg-[#161b22] p-6"
                style={{ borderLeft: '4px solid #a371f7' }}
              >
                <h2
                  style={{
                    margin: '0 0 8px 0',
                    fontSize: '14px',
                    fontWeight: '600',
                    color: '#a371f7',
                  }}
                >
                  Strategy Phase
                </h2>
                <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
                  {currentResult
                    ? 'Rotation triggers evaluated against last simulation.'
                    : 'Run a contract analysis to evaluate rotation triggers.'}
                </p>
              </div>
            )}

            {/* NAV / Performance */}
            {dashboardLoading ? (
              <NAVSkeleton />
            ) : (
              <div
                className="rounded-xl border border-[#30363d] bg-[#161b22] p-6"
                style={{ borderLeft: '4px solid #fb8500' }}
              >
                <h2
                  style={{
                    margin: '0 0 8px 0',
                    fontSize: '14px',
                    fontWeight: '600',
                    color: '#fb8500',
                  }}
                >
                  NAV & Performance
                </h2>
                <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
                  {currentResult
                    ? 'Performance fee accrual updated after last simulation.'
                    : 'Analyse a vault contract to track NAV and fee accrual.'}
                </p>
              </div>
            )}
          </div>

          {/* ── WASM Upload Zone ─────────────────────────────────────────── */}
          <div
            style={{
              backgroundColor: '#161b22',
              borderRadius: '12px',
              padding: '28px',
              marginBottom: '24px',
              border: '1px solid #30363d',
            }}
          >
            <div style={{ marginBottom: '16px' }}>
              <h2
                style={{
                  margin: '0 0 4px 0',
                  fontSize: '16px',
                  fontWeight: '600',
                  color: '#c9d1d9',
                }}
              >
                Upload Contract
              </h2>
              <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
                Drop a compiled Soroban contract (.wasm) to analyse its resource usage
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
                onFileReady={(file) => {
                  console.log('[UploadZone] Contract ready for analysis:', file.name, file.size, 'bytes');
                  setWasmFile(file);
                  const reader = new FileReader();
                  reader.onload = async (e) => {
                    const arrayBuffer = e.target?.result as ArrayBuffer;
                    const base64 = arrayBufferToBase64(arrayBuffer);
                    setWasmData(base64);
                    await handleSimulate({}, base64);
                  };
                  reader.readAsArrayBuffer(file);
                  void handleWasmReady(file);
                }}
                onReset={() => {
                  setWasmFile(null);
                  setWasmData(null);
                  setCurrentResult(null);
                }}
              />
            </ErrorBoundary>
          </div>

          {/* ── Gas Golfing Results ──────────────────────────────────────── */}
          <div style={{ marginBottom: '24px' }}>
            {gasGolfingLoading ? (
              /* Skeleton for the gas golfing suggestions table */
              <div className="rounded-lg border border-[#30363d] bg-[#0d1117] p-4">
                <div className="mb-3 flex items-center justify-between">
                  <Skeleton className="h-4 w-48" />
                  <Skeleton className="h-4 w-20" />
                </div>
                <div className="space-y-2">
                  {[0, 1, 2, 3].map((i) => (
                    <div key={i} className="flex items-center gap-3 py-2">
                      <Skeleton className="h-4 w-16 rounded-full" />
                      <Skeleton className="h-4 flex-1" />
                      <Skeleton className="h-4 w-12" />
                    </div>
                  ))}
                </div>
              </div>
            ) : gasGolfingError ? (
              <div className="rounded-lg border border-[#fb8500] bg-[#0d1117] p-4 text-sm text-[#f0883e]">
                {gasGolfingError}
              </div>
            ) : gasGolfingSuggestions.length ? (
              <GasGolfingSuggestionsTable suggestions={gasGolfingSuggestions} />
            ) : null}
          </div>

          {/* ── Contract ID Input ────────────────────────────────────────── */}
          <div
            style={{
              backgroundColor: '#161b22',
              borderRadius: '8px',
              padding: '24px',
              marginBottom: '24px',
              border: '1px solid #30363d',
            }}
          >
            <label
              style={{
                display: 'block',
                marginBottom: '8px',
                fontWeight: '500',
                color: '#c9d1d9',
              }}
            >
              Contract ID
            </label>
            <input
              type="text"
              value={contractId}
              onChange={(e) => setContractId(e.target.value)}
              placeholder="Enter Soroban contract ID"
              style={{
                width: '100%',
                padding: '12px 16px',
                border: '1px solid #30363d',
                borderRadius: '6px',
                fontSize: '14px',
                fontFamily: 'monospace',
                boxSizing: 'border-box',
                backgroundColor: '#0d1117',
                color: '#c9d1d9',
              }}
            />
            <p style={{ margin: '8px 0 0 0', fontSize: '12px', color: '#8b949e' }}>
              Contract ID:{' '}
              <code style={{ color: '#00d9ff' }}>{contractId.substring(0, 20)}...</code>
            </p>
            {wasmFile && (
              <div
                style={{
                  marginTop: '16px',
                  padding: '12px',
                  backgroundColor: 'rgba(52, 211, 153, 0.08)',
                  border: '1px solid rgba(52, 211, 153, 0.25)',
                  borderRadius: '6px',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '8px',
                }}
              >
                <span style={{ color: '#34d399', fontSize: '12px', fontWeight: '600' }}>
                  Active WASM:
                </span>
                <code style={{ color: '#c9d1d9', fontSize: '12px', fontFamily: 'monospace' }}>
                  {wasmFile.name}
                </code>
                <span style={{ color: '#8b949e', fontSize: '11px' }}>
                  ({(wasmFile.size / 1024).toFixed(1)} KB)
                </span>
              </div>
            )}
          </div>

          {/* ── Function Selection + Results ──────────────────────────────── */}
          <div className="mb-6 grid grid-cols-1 gap-6 lg:grid-cols-2">
            {/* Left Column – Function Selection & Form */}
            <div>
              <FunctionSidebar
                functions={MOCK_CONTRACT_FUNCTIONS}
                selectedFunction={selectedFunction}
                onSelect={(func) => {
                  setSelectedFunction(func);
                  setCurrentResult(null);
                }}
              />
              <ContractInteraction
                selectedFunction={selectedFunction}
                loading={loading}
                onSubmit={handleSimulate}
              />
            </div>

            {/* Right Column – Results & History Tabs */}
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
                    borderBottom: tab === 'explorer' ? '2px solid #00d9ff' : '2px solid transparent',
                    cursor: 'pointer',
                    fontSize: '14px',
                    fontWeight: tab === 'explorer' ? '600' : '500',
                    color: tab === 'explorer' ? '#00d9ff' : '#8b949e',
                    transition: 'color 0.2s, border-bottom-color 0.2s',
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
                    borderBottom:
                      tab === 'history' ? '2px solid #00d9ff' : '2px solid transparent',
                    cursor: 'pointer',
                    fontSize: '14px',
                    fontWeight: tab === 'history' ? '600' : '500',
                    color: tab === 'history' ? '#00d9ff' : '#8b949e',
                    transition: 'color 0.2s, border-bottom-color 0.2s',
                  }}
                >
                  History ({history.length})
                </button>
              </div>

              {/* Tab Content */}
              <div
                style={{
                  backgroundColor: '#161b22',
                  borderRadius: '0 8px 8px 8px',
                  padding: '24px',
                  border: '1px solid #30363d',
                  borderTop: 'none',
                  transition: 'opacity 0.2s',
                  opacity: 1,
                }}
              >
                {tab === 'explorer' ? (
                  loading ? (
                    /* Analysis in-flight: show skeleton for result + nutrition label */
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
                        <button
                          type="button"
                          onClick={() => {
                            setCurrentResult(null);
                            const resetBtn = document.getElementById('wasm-upload-reset-btn');
                            if (resetBtn) resetBtn.click();
                          }}
                          className="mt-4 rounded px-4 py-2 bg-slate-800 text-slate-300 hover:bg-slate-700 transition"
                        >
                          Clear analysis
                        </button>
                      )}

                      {currentResult?.resourceCost && (
                        <div className="mt-4 grid grid-cols-1 lg:grid-cols-2 gap-4">
                          <NutritionLabel
                            cpu_instructions={analysisReport!.cpu_instructions}
                            ram_bytes={analysisReport!.ram_bytes}
                            ledger_read_bytes={analysisReport!.ledger_read_bytes}
                            ledger_write_bytes={analysisReport!.ledger_write_bytes}
                            transaction_size_bytes={analysisReport!.transaction_size_bytes}
                          />
                          <GasUsageChart
                            cpu_instructions={currentResult.resourceCost.cpu_instructions}
                            ram_bytes={currentResult.resourceCost.ram_bytes}
                            ledger_read_bytes={currentResult.resourceCost.ledger_read_bytes}
                            ledger_write_bytes={currentResult.resourceCost.ledger_write_bytes}
                            transaction_size_bytes={
                              currentResult.resourceCost.transaction_size_bytes
                            }
                            cost_stroops={currentResult.resourceCost.cost_stroops}
                            testnetAverages={currentResult.resourceCost.testnet_averages}
                          />
                        </div>
                      )}

                      {currentResult?.resourceCost && (
                        <div className="mt-4">
                          <ResourceHeatmap
                            resourceCost={{
                              cpu_instructions: currentResult.resourceCost.cpu_instructions,
                              ram_bytes: currentResult.resourceCost.ram_bytes,
                              ledger_read_bytes: currentResult.resourceCost.ledger_read_bytes,
                              ledger_write_bytes: currentResult.resourceCost.ledger_write_bytes,
                              transaction_size_bytes:
                                currentResult.resourceCost.transaction_size_bytes,
                              cost_stroops: (currentResult.resourceCost as any).cost_stroops,
                              state_snapshot: currentResult.stateSnapshot,
                            }}
                          />
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
        </main>
      </div>

      {/* Wallet Modal */}
      <WalletModal />
    </>
  );
}
