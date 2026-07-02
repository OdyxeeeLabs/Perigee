'use client';

export function ResultViewerSkeleton() {
  const heatmapCells = Array.from({ length: 36 }, (_, index) => {
    const phase = index % 4;
    const toneClass =
      phase === 0
        ? 'bg-cyan-600/30 border-cyan-500/20'
        : phase === 1
        ? 'bg-slate-700/70 border-slate-600/60'
        : phase === 2
        ? 'bg-amber-500/20 border-amber-500/30'
        : 'bg-slate-800/80 border-slate-700/80';
    return { id: index, toneClass };
  });

  return (
    <div
      style={{
        padding: '24px',
        backgroundColor: '#0d1117',
        borderRadius: '8px',
        borderLeft: '4px solid #00d9ff',
        border: '1px solid #30363d',
      }}
      className="animate-pulse"
    >
      <div style={{ marginBottom: '24px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div className="flex items-center gap-3">
          <div className="relative flex h-3 w-3">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-[#00d9ff] opacity-75"></span>
            <span className="relative inline-flex rounded-full h-3 w-3 bg-[#00d9ff]"></span>
          </div>
          <div>
            <h3
              style={{
                margin: '0 0 4px 0',
                color: '#00d9ff',
                fontSize: '16px',
                fontWeight: '600',
              }}
            >
              Simulating Transaction...
            </h3>
            <p style={{ margin: '0', color: '#8b949e', fontSize: '12px' }}>
              Profiling smart contract resource cost
            </p>
          </div>
        </div>

        <div className="h-8 w-40 bg-[#1f2937] rounded-md border border-[#374151]" />
      </div>

      {/* Code Result Skeleton Box */}
      <div
        style={{
          backgroundColor: '#0d1117',
          padding: '16px',
          borderRadius: '6px',
          marginBottom: '16px',
          border: '1px solid #30363d',
        }}
      >
        <div className="flex flex-col gap-3">
          <div className="h-4 w-24 bg-[#30363d] rounded" />
          <div className="h-3 w-full bg-[#161b22] rounded" />
          <div className="h-3 w-5/6 bg-[#161b22] rounded" />
          <div className="h-3 w-4/5 bg-[#161b22] rounded" />
          <div className="h-3 w-2/3 bg-[#161b22] rounded" />
        </div>
      </div>

      {/* Call Graph Skeleton Box */}
      <div
        style={{
          backgroundColor: '#161b22',
          padding: '20px',
          borderRadius: '8px',
          border: '1px solid #30363d',
          minHeight: '120px',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: '12px',
        }}
      >
        <div className="h-4 w-32 bg-[#30363d] rounded mb-2" />
        <div className="flex items-center gap-4">
          <div className="h-10 w-24 bg-[#0d1117] rounded-lg border border-[#30363d] flex items-center justify-center">
            <div className="h-2 w-12 bg-[#30363d] rounded" />
          </div>
          <div className="h-[2px] w-8 bg-[#30363d] relative">
            <div className="absolute right-0 top-1/2 -translate-y-1/2 border-t-[4px] border-b-[4px] border-l-[6px] border-transparent border-l-[#30363d]" />
          </div>
          <div className="h-10 w-24 bg-[#0d1117] rounded-lg border border-[#30363d] flex items-center justify-center">
            <div className="h-2 w-12 bg-[#30363d] rounded" />
          </div>
        </div>
      </div>

      {/* Heatmap Matrix Skeleton */}
      <div
        style={{
          backgroundColor: '#161b22',
          padding: '20px',
          borderRadius: '8px',
          border: '1px solid #30363d',
          marginTop: '16px',
        }}
      >
        <div className="flex items-center justify-between mb-4">
          <div className="h-4 w-44 bg-[#30363d] rounded" />
          <div className="h-7 w-32 bg-[#0d1117] rounded-md border border-[#30363d]" />
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-[auto_1fr] gap-6">
          <div className="grid grid-cols-6 gap-2 bg-[#0d1117] p-3 rounded-lg border border-[#30363d] w-fit">
            {heatmapCells.map((cell) => (
              <div
                key={cell.id}
                className={`w-8 h-8 rounded border ${cell.toneClass}`}
              />
            ))}
          </div>

          <div className="bg-[#0d1117] rounded-lg border border-[#30363d] p-4 min-h-[170px]">
            <div className="h-3 w-48 bg-[#30363d] rounded mb-4" />
            <div className="h-5 w-36 bg-[#1f2937] rounded mb-3" />
            <div className="h-3 w-full bg-[#1f2937] rounded mb-2" />
            <div className="h-3 w-11/12 bg-[#1f2937] rounded mb-2" />
            <div className="h-3 w-9/12 bg-[#1f2937] rounded" />
          </div>
        </div>

        <div className="mt-4 grid grid-cols-2 md:grid-cols-4 gap-3">
          <div className="h-14 rounded-lg bg-[#0d1117] border border-[#30363d]" />
          <div className="h-14 rounded-lg bg-[#0d1117] border border-[#30363d]" />
          <div className="h-14 rounded-lg bg-[#0d1117] border border-[#30363d]" />
          <div className="h-14 rounded-lg bg-[#0d1117] border border-[#30363d]" />
        </div>
      </div>
    </div>
  );
}
