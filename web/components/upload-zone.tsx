'use client';

import React, { useCallback, useState } from 'react';
import { useDropzone, FileRejection } from 'react-dropzone';

// ─── Types ────────────────────────────────────────────────────────────────────

type UploadState = 'idle' | 'hover' | 'scanning' | 'success' | 'error';

interface DroppedFile {
  name: string;
  sizeBytes: number;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

// ─── Sub-components ───────────────────────────────────────────────────────────

/** Animated WASM hex-grid icon */
function WasmIcon({ state }: { state: UploadState }) {
  const isActive = state === 'hover' || state === 'scanning' || state === 'success';
  return (
    <svg
      width="64"
      height="64"
      viewBox="0 0 64 64"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={`transition-all duration-500 ${isActive ? 'scale-110' : 'scale-100'}`}
    >
      {/* Outer hexagon */}
      <path
        d="M32 4 L56 18 L56 46 L32 60 L8 46 L8 18 Z"
        stroke={
          state === 'error'
            ? '#f87171'
            : state === 'success'
            ? '#34d399'
            : state === 'scanning'
            ? '#a78bfa'
            : state === 'hover'
            ? '#38bdf8'
            : '#334155'
        }
        strokeWidth="2"
        fill={
          state === 'error'
            ? 'rgba(248,113,113,0.08)'
            : state === 'success'
            ? 'rgba(52,211,153,0.08)'
            : state === 'scanning'
            ? 'rgba(167,139,250,0.08)'
            : state === 'hover'
            ? 'rgba(56,189,248,0.08)'
            : 'rgba(30,41,59,0.5)'
        }
        className="transition-all duration-500"
      />
      {/* Inner WASM label */}
      <text
        x="32"
        y="35"
        textAnchor="middle"
        fontSize="11"
        fontWeight="700"
        fontFamily="monospace"
        fill={
          state === 'error'
            ? '#f87171'
            : state === 'success'
            ? '#34d399'
            : state === 'scanning'
            ? '#a78bfa'
            : state === 'hover'
            ? '#38bdf8'
            : '#64748b'
        }
        className="transition-all duration-500"
      >
        .wasm
      </text>
    </svg>
  );
}

/** Pulsing scanning bar animation */
function ScanningAnimation() {
  return (
    <div className="w-full mt-3 overflow-hidden rounded-full h-1 bg-slate-800">
      <div
        className="h-full rounded-full bg-gradient-to-r from-violet-500 via-fuchsia-400 to-violet-500"
        style={{
          animation: 'scan-sweep 1.6s ease-in-out infinite',
          backgroundSize: '200% 100%',
        }}
      />
      <style>{`
        @keyframes scan-sweep {
          0%   { transform: translateX(-100%); }
          100% { transform: translateX(200%); }
        }
      `}</style>
    </div>
  );
}

/** Spinning dots loader */
function SpinnerDots() {
  return (
    <div className="flex gap-1.5 items-center justify-center mt-2">
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="w-1.5 h-1.5 rounded-full bg-violet-400"
          style={{
            animation: `dot-pulse 1.2s ease-in-out ${i * 0.2}s infinite`,
          }}
        />
      ))}
      <style>{`
        @keyframes dot-pulse {
          0%, 80%, 100% { opacity: 0.2; transform: scale(0.8); }
          40%            { opacity: 1;   transform: scale(1.2); }
        }
      `}</style>
    </div>
  );
}

/** Checkmark success indicator */
function SuccessIcon() {
  return (
    <svg
      className="w-5 h-5 text-emerald-400 inline-block mr-1.5"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2.5}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  );
}

/** Error X indicator */
function ErrorIcon() {
  return (
    <svg
      className="w-5 h-5 text-red-400 inline-block mr-1.5"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2.5}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
    </svg>
  );
}

// ─── Main Component ───────────────────────────────────────────────────────────

export interface UploadZoneProps {
  /** Called with the validated File once scanning completes */
  onFileReady?: (file: File) => void;
}

export function UploadZone({ onFileReady }: UploadZoneProps) {
  const [uploadState, setUploadState] = useState<UploadState>('idle');
  const [droppedFile, setDroppedFile] = useState<DroppedFile | null>(null);
  const [errorMessage, setErrorMessage] = useState<string>('');
  const [unexpectedError, setUnexpectedError] = useState<Error | null>(null);

  if (unexpectedError) {
    throw unexpectedError;
  }

  // ── Drop handling ────────────────────────────────────────────────────────────

  const onDropAccepted = useCallback(
    (files: File[]) => {
      const file = files[0];
      setDroppedFile({ name: file.name, sizeBytes: file.size });
      setUploadState('scanning');
      setErrorMessage('');

      const reader = new FileReader();
      reader.onload = (event) => {
        setTimeout(() => {
          try {
            const arrayBuffer = event.target?.result as ArrayBuffer;
            if (!arrayBuffer) throw new Error('Failed to read file content');

            if (arrayBuffer.byteLength < 8) {
              throw new Error('File is too small to be a valid WebAssembly module');
            }

            const view = new DataView(arrayBuffer);
            
            const magicNumber = view.getUint32(0, false);
            if (magicNumber !== 0x0061736d) {
              throw new Error('Invalid WASM magic number. File is not a valid WebAssembly module');
            }

            const version = view.getUint32(4, true);
            if (version !== 1) {
              throw new Error(`Unsupported WASM version: ${version}. Expected version 1`);
            }

            setUploadState('success');
            onFileReady?.(file);
          } catch (error) {
            setErrorMessage(error instanceof Error ? error.message : 'Failed to parse WASM metadata');
            setUploadState('error');
            setDroppedFile(null);
          }
        }, 800);
      };
      
      reader.onerror = () => {
        setErrorMessage(reader.error?.message ?? 'Unable to read the selected file');
        setUploadState('error');
        setDroppedFile(null);
      };

      try {
        reader.readAsArrayBuffer(file);
      } catch (error) {
        setErrorMessage(error instanceof Error ? error.message : 'Unable to start reading the selected file');
        setUploadState('error');
        setDroppedFile(null);
      }
    },
    [onFileReady]
  );

  const onDropRejected = useCallback((rejections: FileRejection[]) => {
    const first = rejections[0];
    const fileName = first?.file?.name ?? 'file';
    const ext = fileName.includes('.') ? `.${fileName.split('.').pop()}` : 'unknown type';
    const customMessage = first?.errors?.[0]?.message;
    setErrorMessage(
      customMessage || `"${fileName}" was rejected — only .wasm files are accepted (got ${ext})`
    );
    setUploadState('error');
    setDroppedFile(null);
  }, []);

  const wasmValidator = useCallback((file: File) => {
    const extension = file.name.split('.').pop()?.toLowerCase();
    if (extension !== 'wasm') {
      return {
        code: 'file-invalid-type',
        message: `"${file.name}" was rejected — only .wasm files are accepted (got .${extension || 'unknown'})`,
      };
    }
    return null;
  }, []);

  // ── Dropzone config ──────────────────────────────────────────────────────────

  const { getRootProps, getInputProps, isDragActive, open } = useDropzone({
    onDropAccepted,
    onDropRejected,
    validator: wasmValidator,
    accept: {
      'application/wasm': ['.wasm'],
      'application/octet-stream': ['.wasm'],
    },
    maxFiles: 1,
    noClick: uploadState === 'scanning',
    noDrag: uploadState === 'scanning',
  });

  // ── Reset ────────────────────────────────────────────────────────────────────

  const handleReset = (e: React.MouseEvent) => {
    e.stopPropagation();
    setUploadState('idle');
    setDroppedFile(null);
    setErrorMessage('');
    setUnexpectedError(null);
  };

  // ── Dynamic border & bg classes ──────────────────────────────────────────────

  const isHovered = isDragActive && uploadState !== 'scanning';
  const displayState = isHovered ? 'hover' : uploadState;

  const borderColor = {
    idle: 'border-slate-600 hover:border-slate-400',
    hover: 'border-sky-400 shadow-[0_0_24px_rgba(56,189,248,0.2)]',
    scanning: 'border-violet-500 shadow-[0_0_24px_rgba(167,139,250,0.25)]',
    success: 'border-emerald-500 shadow-[0_0_24px_rgba(52,211,153,0.2)]',
    error: 'border-red-500 shadow-[0_0_24px_rgba(248,113,113,0.2)]',
  }[displayState];

  const bgColor = {
    idle: 'bg-slate-900/60 hover:bg-slate-800/60',
    hover: 'bg-sky-950/50',
    scanning: 'bg-violet-950/40',
    success: 'bg-emerald-950/40',
    error: 'bg-red-950/30',
  }[displayState];

  // ── Render ───────────────────────────────────────────────────────────────────

  return (
    <div className="w-full font-sans">
      {/* Drop Zone */}
      <div
        id="wasm-upload-zone"
        {...getRootProps()}
        className={[
          'relative flex flex-col items-center justify-center',
          'border-2 border-dashed rounded-2xl p-10',
          'cursor-pointer transition-all duration-300 ease-in-out select-none',
          'min-h-[260px]',
          borderColor,
          bgColor,
        ].join(' ')}
        role="button"
        aria-label="WASM file upload zone"
      >
        <input {...getInputProps()} id="wasm-file-input" aria-label="Upload .wasm file" />

        {/* Animated glow ring on hover */}
        {(displayState === 'hover' || displayState === 'scanning') && (
          <span
            className="absolute inset-0 rounded-2xl pointer-events-none"
            style={{
              boxShadow:
                displayState === 'hover'
                  ? '0 0 0 1px rgba(56,189,248,0.3)'
                  : '0 0 0 1px rgba(167,139,250,0.35)',
              animation: 'pulse-ring 2s ease-in-out infinite',
            }}
          />
        )}

        {/* ── IDLE / HOVER STATE ── */}
        {(displayState === 'idle' || displayState === 'hover') && (
          <div className="flex flex-col items-center text-center gap-4 transition-all duration-300">
            <WasmIcon state={displayState} />
            <div>
              <p
                className={`text-base font-semibold transition-colors duration-300 ${
                  displayState === 'hover' ? 'text-sky-300' : 'text-slate-300'
                }`}
              >
                {displayState === 'hover'
                  ? 'Release to upload your .wasm file'
                  : 'Drag & drop your compiled .wasm file'}
              </p>
              <p className="text-sm text-slate-500 mt-1">
                or{' '}
                <button
                  type="button"
                  className="text-sky-400 underline underline-offset-2 hover:text-sky-300 transition-colors"
                  onClick={(e) => { e.stopPropagation(); open(); }}
                >
                  click to browse
                </button>
              </p>
            </div>
            <div className="flex items-center gap-2 mt-1 px-4 py-1.5 rounded-full bg-slate-800/70 border border-slate-700">
              <span className="w-2 h-2 rounded-full bg-sky-400" />
              <span className="text-xs text-slate-400 font-mono">Only .wasm files accepted</span>
            </div>
          </div>
        )}

        {/* ── SCANNING STATE ── */}
        {uploadState === 'scanning' && (
          <div className="flex flex-col items-center text-center gap-3 w-full px-4">
            <WasmIcon state="scanning" />
            <p className="text-violet-300 font-semibold text-base tracking-wide">
              Scanning contract…
            </p>
            {droppedFile && (
              <div className="flex items-center gap-2 text-xs text-slate-400 font-mono bg-slate-800/70 px-3 py-1.5 rounded-full border border-slate-700">
                <span className="text-violet-400">📄</span>
                <span className="truncate max-w-[240px]">{droppedFile.name}</span>
                <span className="text-slate-500">·</span>
                <span>{formatBytes(droppedFile.sizeBytes)}</span>
              </div>
            )}
            <ScanningAnimation />
            <SpinnerDots />
            <p className="text-xs text-slate-500">Parsing WASM binary · analysing resource usage…</p>
          </div>
        )}

        {/* ── SUCCESS STATE ── */}
        {uploadState === 'success' && droppedFile && (
          <div className="flex flex-col items-center text-center gap-4">
            <WasmIcon state="success" />
            <div>
              <p className="text-emerald-400 font-semibold text-base">
                <SuccessIcon />
                Contract uploaded successfully
              </p>
              <p className="text-xs text-slate-500 mt-1">
                Ready for resource analysis
              </p>
            </div>

            {/* File info card */}
            <div className="flex items-center gap-3 bg-slate-800/80 border border-emerald-700/40 rounded-xl px-5 py-3">
              <div className="w-9 h-9 rounded-lg bg-emerald-900/50 border border-emerald-700 flex items-center justify-center flex-shrink-0">
                <span className="text-emerald-400 text-xs font-bold font-mono">WA</span>
              </div>
              <div className="text-left">
                <p className="text-sm font-medium text-slate-200 truncate max-w-[220px]">
                  {droppedFile.name}
                </p>
                <p className="text-xs text-slate-500 font-mono">{formatBytes(droppedFile.sizeBytes)}</p>
              </div>
            </div>

            <button
              type="button"
              id="wasm-upload-reset-btn"
              onClick={handleReset}
              className="text-xs text-slate-500 hover:text-slate-300 underline underline-offset-2 transition-colors mt-1"
            >
              Upload a different file
            </button>
          </div>
        )}

        {/* ── ERROR STATE ── */}
        {uploadState === 'error' && (
          <div className="flex flex-col items-center text-center gap-4">
            <WasmIcon state="error" />
            <div>
              <p className="text-red-400 font-semibold text-base">
                <ErrorIcon />
                File rejected
              </p>
              <p className="text-xs text-red-300/70 mt-1 max-w-[280px] leading-relaxed">
                {errorMessage}
              </p>
            </div>

            <div className="flex items-center gap-2 px-4 py-1.5 rounded-full bg-red-950/40 border border-red-800/50">
              <span className="w-2 h-2 rounded-full bg-red-400" />
              <span className="text-xs text-red-400 font-mono">Only .wasm files are accepted</span>
            </div>

            <button
              type="button"
              id="wasm-upload-try-again-btn"
              onClick={handleReset}
              className="mt-1 px-5 py-2 rounded-lg bg-slate-800 border border-slate-700 text-sm text-slate-300 hover:bg-slate-700 hover:text-white transition-all"
            >
              Try again
            </button>
          </div>
        )}
      </div>

      {/* Caption hint */}
      <p className="text-xs text-slate-600 text-center mt-3 font-mono">
        WASM Resource Analyzer · Soroscope · compiled Soroban contracts only
      </p>

      <style>{`
        @keyframes pulse-ring {
          0%, 100% { opacity: 0.4; }
          50%       { opacity: 1; }
        }
      `}</style>
    </div>
  );
}
