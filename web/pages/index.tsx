import { WalletModal } from "../components/WalletModal";
import { ConnectButton } from "../components/ConnectButton";
import { useState } from 'react';
import { ResultViewer } from '../components/Resultviewer';
import { InvocationHistory, useInvocationHistory } from '../components/InnovocationHistory';
import { NutritionLabel } from '../components/NutritionLabel';
import { FunctionSidebar } from '../components/FunctionSidebar';
import { ContractInteraction } from '../components/ContractInteraction';
import { MOCK_CONTRACT_FUNCTIONS, generateMockResult, generateMockResourceCost } from '../lib/sorobantypes';
import type { ContractFunction, InvocationResult } from '../lib/sorobantypes';
import { UploadZone } from '../components/upload-zone';
import { extractErrorDetails, createUserFriendlyMessage } from '../lib/errorHandling';
import { ErrorBoundary } from '../components/ErrorBoundary';

export default function Home() {
  const [contractId, setContractId] = useState('CAEZJVJ4N7P7GRUVD5NG5LYYH23AQHJUKQEUHW54LR5PGQX3V7FXD7Q');
  const [selectedFunction, setSelectedFunction] = useState<ContractFunction>(MOCK_CONTRACT_FUNCTIONS[0]);
  const [currentResult, setCurrentResult] = useState<InvocationResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [tab, setTab] = useState<'explorer' | 'history'>('explorer');
  const { history, addToHistory } = useInvocationHistory();

  const handleSimulate = async (inputs: Record<string, any>) => {
    setLoading(true);
    let errorType: string | undefined;
    try {
      const response = await fetch('http://localhost:8080/analyze', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          contract_id: contractId,
          function_name: selectedFunction.name,
        }),
      });

      if (!response.ok) {
        // Parse error response from backend
        const errorResponse = await extractErrorDetails(response);
        errorType = errorResponse.error;
        const userMessage = createUserFriendlyMessage(errorResponse);
        throw new Error(userMessage);
      }

      const report = await response.json();

      const result: InvocationResult = {
        id: Math.random().toString(36).substring(7),
        functionName: selectedFunction.name,
        inputs,
        result: generateMockResult(selectedFunction.name, inputs),
        resourceCost: report,
        timestamp: Date.now(),
        success: true,
      };

      setCurrentResult(result);
      addToHistory(result);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'An unexpected error occurred during analysis';
      
      const errorResult: InvocationResult = {
        id: Math.random().toString(36).substring(7),
        functionName: selectedFunction.name,
        inputs,
        error: errorMessage,
        errorType: errorType || 'UNKNOWN_ERROR',
        timestamp: Date.now(),
        success: false,
      };
      setCurrentResult(errorResult);
      addToHistory(errorResult);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ minHeight: '100vh', backgroundColor: '#0f1117' }}>
      {/* Header */}
      <header
        style={{
          backgroundColor: '#1a1f26',
          borderBottom: '1px solid #30363d',
          padding: '24px 0',
          position: 'sticky',
          top: 0,
          zIndex: 100,
          display: 'flex',
          justifyContent: 'space-between'
        }}
      >
        <div style={{ maxWidth: '1200px', paddingLeft: '140px' }}>
          <h1 style={{ margin: '0 0 12px 0', fontSize: '28px', fontWeight: '700', color: '#00d9ff', letterSpacing: '0.5px' }}>
            SoroScope
          </h1>
          <p style={{ margin: '0', color: '#8b949e', fontSize: '14px' }}>
            Explore and test Soroban smart contracts with precision
          </p>
        </div>

        {/* Wallet Connection in Top-Right */}
        <div className="pr-[125px]">
          <ConnectButton />
        </div>
      </header>

      {/* Main Content */}
      <main style={{ maxWidth: '1200px', margin: '0 auto', padding: '24px' }}>

        {/* WASM Upload Zone */}
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
            <h2 style={{ margin: '0 0 4px 0', fontSize: '16px', fontWeight: '600', color: '#c9d1d9' }}>
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
                // TODO: wire into your analysis flow, e.g. POST file bytes to /analyze
              }}
            />
          </ErrorBoundary>
        </div>

        {/* Contract ID Input */}
        <div
          style={{
            backgroundColor: '#161b22',
            borderRadius: '8px',
            padding: '24px',
            marginBottom: '24px',
            border: '1px solid #30363d',
          }}
        >
          <label style={{ display: 'block', marginBottom: '8px', fontWeight: '500', color: '#c9d1d9' }}>
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
            Contract ID: <code style={{ color: '#00d9ff' }}>{contractId.substring(0, 20)}...</code>
          </p>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '24px', marginBottom: '24px' }}>
          {/* Left Column - Function Selection & Form */}
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
              }}
            >
              {tab === 'explorer' ? (
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
              ) : (
                <InvocationHistory onSelectResult={(result) => {
                  setCurrentResult(result);
                  setTab('explorer');
                }} />
              )}
            </div>
          </div>
        </div>

        {/* Info Cards */}
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
            gap: '16px',
          }}
        >
          <div
            style={{
              backgroundColor: '#161b22',
              borderRadius: '8px',
              padding: '16px',
              borderLeft: '4px solid #00d9ff',
              border: '1px solid #30363d',
            }}
          >
            <h3
              style={{
                margin: '0 0 8px 0',
                fontSize: '14px',
                fontWeight: '600',
                color: '#00d9ff',
              }}
            >
              Simulate
            </h3>
            <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
              Preview contract execution without signing or spending XLM
            </p>
          </div>

          <div
            style={{
              backgroundColor: '#161b22',
              borderRadius: '8px',
              padding: '16px',
              borderLeft: '4px solid #a371f7',
              border: '1px solid #30363d',
            }}
          >
            <h3
              style={{
                margin: '0 0 8px 0',
                fontSize: '14px',
                fontWeight: '600',
                color: '#a371f7',
              }}
            >
              Invoke
            </h3>
            <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
              Execute real transactions via your connected wallet (Freighter/xBull)
            </p>
          </div>

          <div
            style={{
              backgroundColor: '#161b22',
              borderRadius: '8px',
              padding: '16px',
              borderLeft: '4px solid #fb8500',
              border: '1px solid #30363d',
            }}
          >
            <h3
              style={{
                margin: '0 0 8px 0',
                fontSize: '14px',
                fontWeight: '600',
                color: '#fb8500',
              }}
            >
              History
            </h3>
            <p style={{ margin: '0', fontSize: '13px', color: '#8b949e' }}>
              Track all function calls with full details and resource costs
            </p>
          </div>
        </div>
      </main>
      {/* Wallet Modal */}
      <WalletModal />
    </div>
  );
}
