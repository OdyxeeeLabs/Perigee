'use client';

import type { InvocationResult } from '../lib/sorobantypes';

import { CallGraphVisualizer } from './CallGraphVisualizer';

interface ResultViewerProps {
  result: InvocationResult | null;
}

export function ResultViewer({ result }: ResultViewerProps) {
  const downloadSnapshot = () => {
    if (!result?.stateSnapshot) return;
    const blob = new Blob([JSON.stringify(result.stateSnapshot, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `soroscope-snapshot-${result.functionName}-${Date.now()}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  if (!result) {
    return (
      <div
        style={{
          padding: '24px',
          backgroundColor: '#0d1117',
          borderRadius: '8px',
          textAlign: 'center',
          color: '#8b949e',
          border: '1px solid #30363d',
        }}
      >
        <p>No results yet. Execute a contract function to see results here.</p>
      </div>
    );
  }

  return (
    <div
      style={{
        padding: '24px',
        backgroundColor: '#0d1117',
        borderRadius: '8px',
        borderLeft: `4px solid ${result.success ? '#00d9ff' : '#fb8500'}`,
        border: `1px solid #30363d`,
      }}
    >
      <div style={{ marginBottom: '16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div>
          <h3
            style={{
              margin: '0 0 4px 0',
              color: result.success ? '#00d9ff' : '#fb8500',
              fontSize: '16px',
              fontWeight: '600',
            }}
          >
            {result.success ? '✓ Success' : '✗ Error'}
          </h3>
          <p style={{ margin: '0', color: '#8b949e', fontSize: '12px' }}>
            {new Date(result.timestamp).toLocaleString()}
          </p>
        </div>
        
        {result.stateSnapshot && (
          <button
            onClick={downloadSnapshot}
            style={{
              padding: '6px 12px',
              backgroundColor: '#1f2937',
              color: '#f3f4f6',
              borderRadius: '6px',
              border: '1px solid #374151',
              fontSize: '12px',
              cursor: 'pointer',
              transition: 'background-color 0.2s',
            }}
            onMouseOver={(e) => (e.currentTarget.style.backgroundColor = '#374151')}
            onMouseOut={(e) => (e.currentTarget.style.backgroundColor = '#1f2937')}
          >
            Download State Snapshot
          </button>
        )}
      </div>

      {result.error ? (
        <div
          style={{
            backgroundColor: '#0d1117',
            padding: '12px',
            borderRadius: '6px',
            marginBottom: '12px',
            fontSize: '13px',
            color: '#fb8500',
            fontFamily: 'monospace',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
            border: '1px solid #30363d',
          }}
        >
          {result.error}
        </div>
      ) : (
        result.result && (
          <div
            style={{
              backgroundColor: '#0d1117',
              padding: '12px',
              borderRadius: '6px',
              marginBottom: '12px',
              fontSize: '13px',
              fontFamily: 'monospace',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-all',
              color: '#58a6ff',
              border: '1px solid #30363d',
              maxHeight: '200px',
              overflow: 'auto',
            }}
          >
            <strong style={{ color: '#8b949e' }}>Result:</strong>
            <br />
            {JSON.stringify(result.result, null, 2)}
          </div>
        )
      )}

      {result.callGraphMermaid && (
        <CallGraphVisualizer mermaidDefinition={result.callGraphMermaid} />
      )}
    </div>
  );
}
