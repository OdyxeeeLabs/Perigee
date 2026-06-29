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
    const restored = loadLatestAnalysis();
    if (restored) {
      setCurrentResult(restored);
    }
  }, []);

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
    setCurrentResult(null);
  }, []);

  const handleSimulate = async (inputs: Record<string, any>, customWasmData?: string) => {
    setLoading(true);
    try {
      const url = activeWasmData ? 'http://localhost:8080/analyze/wasm' : 'http://localhost:8080/analyze';
      const body = activeWasmData
        ? { wasm_bytes: activeWasmData, function_name: selectedFunction.name, args: Object.values(inputs).map(val => String(val)) }
        : { contract_id: contractId, function_name: selectedFunction.name };

      const response = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });

      if (!response.ok) {
        throw new Error(`Backend error: ${response.statusText}`);
      }

      const report: AnalyzeResponse = await response.json();
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
      if (error instanceof ApiError) {
        errorType = error.body?.error;
      }
      const formatted = formatError(error);
      const errorResult: InvocationResult = {
        id: Math.random().toString(36).substring(7),
      const message = error instanceof Error ? error.message : "Analysis failed";
      setCurrentResult({
        id: Math.random().toString(36).slice(2),
        functionName: selectedFunction.name,
        inputs,
        error: message,
        errorType: "ANALYSIS_ERROR",
        timestamp: Date.now(),
        success: false,
      };
      setCurrentResult(errorResult);
      addToHistory(errorResult);
    } finally {
      setLoading(false);
    }
  };

  const handleFileUpload = async (file: File) => {
    setLoading(true);
    let errorType: string | undefined;
    try {
      const arrayBuffer = await file.arrayBuffer();
      const response = await fetch('http://localhost:8080/analyze', {
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
      const errorMessage = error instanceof Error ? error.message : 'An unexpected error occurred during analysis';
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
      });
    } finally {
      setLoading(false);
    }
  };

  const handleWasmReady = async (file: File) => {
    setGasGolfingLoading(true);
    setGasGolfingError(null);
    setGasGolfingSuggestions([]);
    try {
      const bytes = await file.arrayBuffer();
      const wasmBytes = arrayBufferToBase64(bytes);
      const res = await fetch('http://localhost:8080/analyze/gas-golfing', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ wasm_bytes: wasmBytes, contract_name: file.name.replace(/\\.wasm$/i, '') }),
      });
      if (!res.ok) {
        const err = await extractErrorDetails(res);
        throw new Error(createUserFriendlyMessage(err));
      }
      const data = await res.json();
      setGasGolfingSuggestions((data?.report?.suggestions ?? []) as GasGolfingSuggestion[]);
    } catch (e) {
      setGasGolfingError(e instanceof Error ? e.message : 'Failed to analyze WASM');
    } finally {
      setGasGolfingLoading(false);
    }
  };

  const analysisReport = currentResult?.analysisReport ?? currentResult?.resourceCost;

  return (
    <>
      <Head>
        <title>SoroScope - Soroban Smart Contract Resource Analyzer</title>
        <meta name="description" content="Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts with absolute precision, utilizing live node queries and direct WASM bytecode analysis." />
        <meta name="viewport" content="width=device-width, initial-scale=1.0" />
        <link rel="icon" href="/favicon.ico" />
      </Head>
      <div style={{ minHeight: '100vh', backgroundColor: '#0f1117' }}>
        <header className="sticky top-0 z-[100] flex flex-col gap-4 border-b border-[#30363d] bg-[#1a1f26] px-6 py-6 sm:flex-row sm:items-center sm:justify-between sm:px-10 lg:pl-[140px] lg:pr-[125px]">
          <div className="max-w-[1200px]">
            <h1 style={{ margin: '0 0 12px 0', fontSize: '28px', fontWeight: '700', color: '#00d9ff', letterSpacing: '0.5px' }}>SoroScope</h1>
            <p style={{ margin: '0', color: '#8b949e', fontSize: '14px' }}>Explore and test Soroban smart contracts with precision</p>
          </div>
          <div><ConnectButton /></div>
        </header>

        <main className="mx-auto max-w-[1200px] px-4 py-6 sm:px-6">
          <div style={{ backgroundColor: '#161b22', borderRadius: '12px', padding: '28px', marginBottom: '24px', border: '1px solid #30363d' }}>
            <div style={{ marginBottom: '16px' }}>
              <h2 style={{ margin: '0 0 4px 0', fontSize: '16px', fontWeight: '600', color: '#c9d1d9' }}>Upload Contract</h2>
              <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>Drop a compiled Soroban contract (.wasm) to analyse its resource usage</p>
            </div>
            <ErrorBoundary
              fallback={(error, reset) => (
                <div className="rounded-lg border border-red-800/60 bg-red-950/30 p-6 text-center text-red-100">
                  <p className="text-sm font-semibold">Upload failed unexpectedly</p>
                  <p className="mx-auto mt-2 max-w-md text-xs leading-relaxed text-red-200/80">{error.message}</p>
                  <button type="button" onClick={reset} className="mt-4 rounded-md border border-red-700/70 px-4 py-2 text-sm text-red-100 hover:bg-red-900/40">Try another file</button>
                </div>
              )}
            >
              <UploadZone
                backendUrl="http://localhost:8080/analyze/wasm"
                enableBackendValidation={true}
                onFileReady={(file) => {
                  setWasmFile(file);
                  const reader = new FileReader();
                  reader.onload = async (e) => {
                    const arrayBuffer = e.target?.result as ArrayBuffer;
                    const bytes = new Uint8Array(arrayBuffer);
                    let binary = '';
                    for (let i = 0; i < bytes.byteLength; i++) {
                      binary += String.fromCharCode(bytes[i]);
                    }
                    const base64 = window.btoa(binary);
                    setWasmData(base64);
                    await handleSimulate({}, base64);
                  };
                  reader.readAsArrayBuffer(file);
                  void handleFileUpload(file);
                  void handleWasmReady(file);
                }}
              />
            </ErrorBoundary>
          </div>

          <div style={{ marginBottom: '24px' }}>
            {gasGolfingLoading ? (
              <div className="rounded-lg border border-[#30363d] bg-[#0d1117] p-4 text-sm text-[#8b949e]">Analyzing WASM for Gas Golfing suggestions…</div>
            ) : gasGolfingError ? (
              <div className="rounded-lg border border-[#fb8500] bg-[#0d1117] p-4 text-sm text-[#f0883e]">{gasGolfingError}</div>
            ) : gasGolfingSuggestions.length ? (
              <GasGolfingSuggestionsTable suggestions={gasGolfingSuggestions} />
            ) : null}
          </div>

          <div style={{ backgroundColor: '#161b22', borderRadius: '8px', padding: '24px', marginBottom: '24px', border: '1px solid #30363d' }}>
            <label style={{ display: 'block', marginBottom: '8px', fontWeight: '500', color: '#c9d1d9' }}>Contract ID</label>
            <input type="text" value={contractId} onChange={(e) => setContractId(e.target.value)} placeholder="Enter Soroban contract ID"
              style={{ width: '100%', padding: '12px 16px', border: '1px solid #30363d', borderRadius: '6px', fontSize: '14px', fontFamily: 'monospace', boxSizing: 'border-box', backgroundColor: '#0d1117', color: '#c9d1d9' }}
            />
            <p style={{ margin: '8px 0 0 0', fontSize: '12px', color: '#8b949e' }}>
              Contract ID: <code style={{ color: '#00d9ff' }}>{contractId.substring(0, 20)}...</code>
            </p>
            {wasmFile && (
              <div style={{ marginTop: '16px', padding: '12px', backgroundColor: 'rgba(52, 211, 153, 0.08)', border: '1px solid rgba(52, 211, 153, 0.25)', borderRadius: '6px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                <span style={{ color: '#34d399', fontSize: '12px', fontWeight: '600' }}>Active WASM:</span>
                <code style={{ color: '#c9d1d9', fontSize: '12px', fontFamily: 'monospace' }}>{wasmFile.name}</code>
                <span style={{ color: '#8b949e', fontSize: '11px' }}>({(wasmFile.size / 1024).toFixed(1)} KB)</span>
              </div>
            )}
          </div>

          <div className="mb-6 grid grid-cols-1 gap-6 lg:grid-cols-2">
            <div>
              <FunctionSidebar functions={MOCK_CONTRACT_FUNCTIONS} selectedFunction={selectedFunction}
                onSelect={(func) => { setSelectedFunction(func); setCurrentResult(null); }}
              />
              <ContractInteraction selectedFunction={selectedFunction} loading={loading} onSubmit={handleSimulate} />
            </div>
            <div>
              <div style={{ display: 'flex', borderBottom: '1px solid #30363d', marginBottom: '24px', backgroundColor: '#161b22', borderRadius: '8px 8px 0 0', gap: '0' }}>
                <button onClick={() => setTab('explorer')}
                  style={{ flex: 1, padding: '12px 16px', backgroundColor: 'transparent', border: 'none', borderBottom: tab === 'explorer' ? '2px solid #00d9ff' : '2px solid transparent', cursor: 'pointer', fontSize: '14px', fontWeight: tab === 'explorer' ? '600' : '500', color: tab === 'explorer' ? '#00d9ff' : '#8b949e', transition: 'color 0.2s, border-bottom-color 0.2s' }}
                >Result</button>
                <button onClick={() => setTab('history')}
                  style={{ flex: 1, padding: '12px 16px', backgroundColor: 'transparent', border: 'none', borderBottom: tab === 'history' ? '2px solid #00d9ff' : '2px solid transparent', cursor: 'pointer', fontSize: '14px', fontWeight: tab === 'history' ? '600' : '500', color: tab === 'history' ? '#00d9ff' : '#8b949e', transition: 'color 0.2s, border-bottom-color 0.2s' }}
                >History ({history.length})</button>
              </div>
              <div style={{ backgroundColor: '#161b22', borderRadius: '0 8px 8px 8px', padding: '24px', border: '1px solid #30363d', borderTop: 'none', transition: 'opacity 0.2s', opacity: 1 }}>
                {tab === 'explorer' ? (
                  loading ? (
                    <>
                      <ResultViewerSkeleton />
                      <div className="mt-4"><NutritionLabelSkeleton /></div>
                    </>
                  ) : currentResult ? (
                    <>
                      <ResultViewer result={currentResult} />
                      {analysisReport && (
                        <button type="button" onClick={() => { setCurrentResult(null); const resetBtn = document.getElementById('wasm-upload-reset-btn'); if (resetBtn) resetBtn.click(); }}
                          className="mt-4 px-4 py-2 bg-slate-800 text-slate-300 rounded hover:bg-slate-700 transition"
                        >Clear analysis</button>
                      )}
                      {currentResult?.resourceCost && (
                        <div className="mt-4 flex flex-col gap-4">
                          <ResourceHeatmap resourceCost={{
                            cpu_instructions: currentResult.resourceCost.cpu_instructions,
                            ram_bytes: currentResult.resourceCost.ram_bytes,
                            ledger_read_bytes: currentResult.resourceCost.ledger_read_bytes,
                            ledger_write_bytes: currentResult.resourceCost.ledger_write_bytes,
                            transaction_size_bytes: currentResult.resourceCost.transaction_size_bytes,
                            cost_stroops: (currentResult.resourceCost as any).cost_stroops,
                            state_snapshot: currentResult.stateSnapshot,
                          }} />
                          <div className="mt-4 grid grid-cols-1 lg:grid-cols-2 gap-4">
                            <NutritionLabel cpu_instructions={analysisReport.cpu_instructions} ram_bytes={analysisReport.ram_bytes}
                              ledger_read_bytes={analysisReport.ledger_read_bytes} ledger_write_bytes={analysisReport.ledger_write_bytes}
                              transaction_size_bytes={analysisReport.transaction_size_bytes}
                            />
                            <GasUsageChart cpu_instructions={currentResult.resourceCost.cpu_instructions}
                              ram_bytes={currentResult.resourceCost.ram_bytes} ledger_read_bytes={currentResult.resourceCost.ledger_read_bytes}
                              ledger_write_bytes={currentResult.resourceCost.ledger_write_bytes} transaction_size_bytes={currentResult.resourceCost.transaction_size_bytes}
                              cost_stroops={currentResult.resourceCost.cost_stroops} testnetAverages={currentResult.resourceCost.testnet_averages}
                            />
                          </div>
                        </div>
                      )}
                    </>
                  ) : (
                    <p style={{ color: '#8b949e', fontSize: '14px' }}>Select a function to simulate</p>
                  )
                ) : (
                  <InvocationHistory onSelectResult={(result) => { setCurrentResult(result); setTab('explorer'); }} />
                )}
              </div>
            </div>
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))', gap: '16px' }}>
            <div style={{ backgroundColor: '#161b22', borderRadius: '8px', padding: '16px', borderLeft: '4px solid #00d9ff', border: '1px solid #30363d' }}>
              <h3 style={{ margin: '0 0 8px 0', fontSize: '14px', fontWeight: '600', color: '#00d9ff' }}>Simulate</h3>
              <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>Preview contract execution without signing or spending XLM</p>
            </div>
            <div style={{ backgroundColor: '#161b22', borderRadius: '8px', padding: '16px', borderLeft: '4px solid #a371f7', border: '1px solid #30363d' }}>
              <h3 style={{ margin: '0 0 8px 0', fontSize: '14px', fontWeight: '600', color: '#a371f7' }}>Invoke</h3>
              <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>Execute real transactions via your connected wallet (Freighter/xBull)</p>
            </div>
            <div style={{ backgroundColor: '#161b22', borderRadius: '8px', padding: '16px', borderLeft: '4px solid #fb8500', border: '1px solid #30363d' }}>
              <h3 style={{ margin: '0 0 8px 0', fontSize: '14px', fontWeight: '600', color: '#fb8500' }}>History</h3>
              <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>Track all function calls with full details and resource costs</p>
            </div>
          </div>
        </main>
        <WalletModal />
      </div>
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

            <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
              <ResultViewer result={currentResult} />
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
