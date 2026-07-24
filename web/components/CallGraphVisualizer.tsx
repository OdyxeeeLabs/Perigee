'use client';

import React, { useEffect, useRef } from 'react';

interface CallGraphVisualizerProps {
  mermaidDefinition: string;
}

export function CallGraphVisualizer({ mermaidDefinition }: CallGraphVisualizerProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let isMounted = true;

    const renderMermaid = async () => {
      if (containerRef.current && mermaidDefinition) {
        try {
          const mermaidModule = await import('mermaid');
          const mermaid = mermaidModule.default || mermaidModule;
          mermaid.initialize({
            startOnLoad: false,
            theme: 'dark',
            securityLevel: 'loose',
            flowchart: {
              useMaxWidth: true,
              htmlLabels: true,
              curve: 'basis',
            },
          });

          if (!isMounted || !containerRef.current) return;
          containerRef.current.innerHTML = '';
          const id = `mermaid-graph-${Math.random().toString(36).substring(2, 9)}`;
          const { svg } = await mermaid.render(id, mermaidDefinition);
          if (isMounted && containerRef.current) {
            containerRef.current.innerHTML = svg;
          }
        } catch (error) {
          console.error('Mermaid rendering failed:', error);
          if (isMounted && containerRef.current) {
            containerRef.current.innerHTML = `<p style="color: #fb8500;">Failed to render call graph: ${String(error)}</p>`;
          }
        }
      }
    };

    renderMermaid();

    return () => {
      isMounted = false;
    };
  }, [mermaidDefinition]);

  return (
    <div style={{ marginTop: '20px' }}>
      <h4 style={{ color: '#00d9ff', fontSize: '14px', marginBottom: '12px', fontWeight: '600' }}>
        Cross-Contract Dependency Graph
      </h4>
      <div
        ref={containerRef}
        style={{
          backgroundColor: '#010409',
          padding: '16px',
          borderRadius: '8px',
          border: '1px solid #30363d',
          overflow: 'auto',
          minHeight: '100px',
        }}
      />
    </div>
  );
}

