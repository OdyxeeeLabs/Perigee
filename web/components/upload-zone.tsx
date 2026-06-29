'use client';

import React, { useCallback, useState } from 'react';
import { useDropzone } from 'react-dropzone';
import type { FileRejection } from 'react-dropzone';

type UploadState = 'idle' | 'hover' | 'scanning' | 'success' | 'error' | 'submitting';

interface DroppedFile {
  name: string;
  sizeBytes: number;
}

interface ErrorDetails {
  title: string;
  message: string;
  suggestedAction?: string;
}

export interface UploadZoneProps {
  onFileReady?: (file: File) => void;
  backendUrl?: string;
  enableBackendValidation?: boolean;
  onReset?: () => void;
}

const MAX_WASM_SIZE = 5 * 1024 * 1024;
const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function hasWasmMagic(buffer: ArrayBuffer): boolean {
  if (buffer.byteLength < WASM_MAGIC.length) return false;
  const bytes = new Uint8Array(buffer, 0, WASM_MAGIC.length);
  return WASM_MAGIC.every((byte, index) => bytes[index] === byte);
}

function WasmIcon({ state }: { state: UploadState }) {
  const isActive = state === 'hover' || state === 'scanning' || state === 'success' || state === 'submitting';
  const accent =
    state === 'error'
      ? '#f87171'
      : state === 'success'
      ? '#34d399'
      : state === 'hover'
      ? '#38bdf8'
      : state === 'scanning' || state === 'submitting'
      ? '#a78bfa'
      : '#64748b';

  return (
    <svg
      width="64"
      height="64"
      viewBox="0 0 64 64"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={`transition-all duration-500 ${isActive ? 'scale-110' : 'scale-100'}`}
    >
      <path
        d="M32 4 L56 18 L56 46 L32 60 L8 46 L8 18 Z"
        stroke={accent}
        strokeWidth="2"
        fill="rgba(30,41,59,0.5)"
        className="transition-all duration-500"
      />
      <text
        x="32"
        y="35"
        textAnchor="middle"
        fontSize="11"
        fontWeight="700"
        fontFamily="monospace"
        fill={accent}
        className="transition-all duration-500"
      >
        .wasm
      </text>
    </svg>
  );
}

function ScanningAnimation() {
  return (
    <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-slate-800">
      <div
        className="h-full rounded-full bg-gradient-to-r from-violet-500 via-fuchsia-400 to-violet-500"
        style={{
          animation: 'scan-sweep 1.6s ease-in-out infinite',
          backgroundSize: '200% 100%',
        }}
      />
      <style>{`
        @keyframes scan-sweep {
          0% { transform: translateX(-100%); }
          100% { transform: translateX(200%); }
        }
      `}</style>
    </div>
  );
}

function SpinnerDots() {
  return (
    <div className="mt-2 flex items-center justify-center gap-1.5">
      {[0, 1, 2].map((index) => (
        <span
          key={index}
          className="h-1.5 w-1.5 rounded-full bg-violet-400"
          style={{
            animation: `dot-pulse 1.2s ease-in-out ${index * 0.2}s infinite`,
          }}
        />
      ))}
      <style>{`
        @keyframes dot-pulse {
          0%, 80%, 100% { opacity: 0.2; transform: scale(0.8); }
          40% { opacity: 1; transform: scale(1.2); }
        }
      `}</style>
    </div>
  );
}

function SuccessIcon() {
  return (
    <svg className="mr-1.5 inline-block h-5 w-5 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  );
}

function ErrorIcon() {
  return (
    <svg className="mr-1.5 inline-block h-5 w-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
    </svg>
  );
}

export function UploadZone({
  onFileReady,
  backendUrl = 'http://localhost:8080/analyze/wasm',
  enableBackendValidation = false,
  onReset,
}: UploadZoneProps) {
  const [uploadState, setUploadState] = useState<UploadState>('idle');
  const [droppedFile, setDroppedFile] = useState<DroppedFile | null>(null);
  const [errorDetails, setErrorDetails] = useState<ErrorDetails | null>(null);

  const setError = useCallback((title: string, message: string, suggestedAction?: string) => {
    setErrorDetails({ title, message, suggestedAction });
    setUploadState('error');
    setDroppedFile(null);
  }, []);

  const submitToBackend = useCallback(
    async (file: File): Promise<void> => {
      setUploadState('submitting');
      const buffer = await file.arrayBuffer();
      const bytes = new Uint8Array(buffer);
      let binary = '';

      for (let index = 0; index < bytes.byteLength; index += 1) {
        binary += String.fromCharCode(bytes[index]);
      }

      const response = await fetch(backendUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          wasm_bytes: btoa(binary),
          function_name: 'main',
          args: [],
        }),
      });

      if (!response.ok) {
        const message = await response.text();
        throw new Error(message || `Backend validation failed with ${response.status}`);
      }
    },
    [backendUrl]
  );

  const handleAcceptedFile = useCallback(
    async (file: File) => {
      setDroppedFile({ name: file.name, sizeBytes: file.size });
      setUploadState('scanning');
      setErrorDetails(null);

      try {
        const buffer = await file.arrayBuffer();

        if (!hasWasmMagic(buffer)) {
          throw new Error(`"${file.name}" is not a valid WASM binary`);
        }

        if (enableBackendValidation) {
          await submitToBackend(file);
        }

        setUploadState('success');
        onFileReady?.(file);
      } catch (error) {
        setError(
          'File rejected',
          error instanceof Error ? error.message : 'Unable to read the selected WASM file',
          'Please upload a compiled Soroban .wasm file.'
        );
      }
    },
    [enableBackendValidation, onFileReady, setError, submitToBackend]
  );

  const onDropAccepted = useCallback(
    (files: File[]) => {
      const file = files[0];
      if (file) {
        void handleAcceptedFile(file);
      }
    },
    [handleAcceptedFile]
  );

  const onDropRejected = useCallback(
    (rejections: FileRejection[]) => {
      const first = rejections[0];
      const fileName = first?.file?.name ?? 'file';
      const isTooLarge = first?.errors.some((error) => error.code === 'file-too-large');
      const extension = fileName.includes('.') ? `.${fileName.split('.').pop()}` : 'unknown type';
      const message = isTooLarge
        ? `"${fileName}" exceeds the ${MAX_WASM_SIZE / (1024 * 1024)} MB size limit`
        : `"${fileName}" was rejected: only .wasm files are accepted (got ${extension})`;

      setError('Invalid file type', message, 'Drop a compiled Soroban .wasm file.');
    },
    [setError]
  );

  const handleDragEnter = useCallback(() => {
    if (uploadState !== 'scanning' && uploadState !== 'submitting') {
      setUploadState('hover');
    }
  }, [uploadState]);

  const handleDragLeave = useCallback(() => {
    if (uploadState === 'hover') {
      setUploadState('idle');
    }
  }, [uploadState]);

  const handleDrop = useCallback(() => {
    if (uploadState === 'hover') {
      setUploadState('scanning');
    }
  }, [uploadState]);

  const wasmValidator = useCallback((file: File) => {
    if (!file.name.toLowerCase().endsWith('.wasm')) {
      return {
        code: 'file-invalid-type',
        message: `"${file.name}" is not a .wasm file`,
      };
    }

    return null;
  }, []);

  const { getRootProps, getInputProps, isDragActive, open } = useDropzone({
    onDropAccepted,
    onDropRejected,
    onDragEnter: handleDragEnter,
    onDragLeave: handleDragLeave,
    onDrop: handleDrop,
    validator: wasmValidator,
    accept: {
      'application/wasm': ['.wasm'],
      'application/octet-stream': ['.wasm'],
    },
    maxFiles: 1,
    maxSize: MAX_WASM_SIZE,
    noClick: uploadState === 'scanning' || uploadState === 'submitting',
    noDrag: uploadState === 'scanning' || uploadState === 'submitting',
  });

  const handleReset = (event: React.MouseEvent) => {
    event.stopPropagation();
    setUploadState('idle');
    setDroppedFile(null);
    setErrorDetails(null);
    onReset?.();
  };

  const displayState = isDragActive && uploadState !== 'scanning' && uploadState !== 'submitting' ? 'hover' : uploadState;

  const borderColor = {
    idle: 'border-slate-600 hover:border-slate-400',
    hover: 'border-sky-400 shadow-[0_0_24px_rgba(56,189,248,0.2)]',
    scanning: 'border-violet-500 shadow-[0_0_24px_rgba(167,139,250,0.25)]',
    submitting: 'border-violet-500 shadow-[0_0_24px_rgba(167,139,250,0.25)]',
    success: 'border-emerald-500 shadow-[0_0_24px_rgba(52,211,153,0.2)]',
    error: 'border-red-500 shadow-[0_0_24px_rgba(248,113,113,0.2)]',
  }[displayState];

  const bgColor = {
    idle: 'bg-slate-900/60 hover:bg-slate-800/60',
    hover: 'bg-sky-950/50',
    scanning: 'bg-violet-950/40',
    submitting: 'bg-violet-950/40',
    success: 'bg-emerald-950/40',
    error: 'bg-red-950/30',
  }[displayState];

  return (
    <div className="w-full font-sans">
      <div
        id="wasm-upload-zone"
        {...getRootProps()}
        className={[
          'relative flex min-h-[260px] flex-col items-center justify-center',
          'rounded-2xl border-2 border-dashed p-10',
          'cursor-pointer select-none transition-all duration-300 ease-in-out',
          borderColor,
          bgColor,
        ].join(' ')}
        role="button"
        aria-label="WASM file upload zone"
      >
        <input {...getInputProps()} id="wasm-file-input" aria-label="Upload .wasm file" />

        {(displayState === 'hover' || displayState === 'scanning' || displayState === 'submitting') && (
          <span
            className="pointer-events-none absolute inset-0 rounded-2xl"
            style={{
              boxShadow:
                displayState === 'hover'
                  ? '0 0 0 1px rgba(56,189,248,0.3)'
                  : '0 0 0 1px rgba(167,139,250,0.35)',
              animation: 'pulse-ring 2s ease-in-out infinite',
            }}
          />
        )}

        {(displayState === 'idle' || displayState === 'hover') && (
          <div className="flex flex-col items-center gap-4 text-center transition-all duration-300">
            <WasmIcon state={displayState} />
            <div>
              <p className={`text-base font-semibold transition-colors duration-300 ${displayState === 'hover' ? 'text-sky-300' : 'text-slate-300'}`}>
                {displayState === 'hover' ? 'Release to upload your .wasm file' : 'Drag & drop your compiled .wasm file'}
              </p>
              <p className="mt-1 text-sm text-slate-500">
                or{' '}
                <button
                  type="button"
                  className="text-sky-400 underline underline-offset-2 transition-colors hover:text-sky-300"
                  onClick={(event) => {
                    event.stopPropagation();
                    open();
                  }}
                >
                  click to browse
                </button>
              </p>
            </div>
            <div className="mt-1 flex items-center gap-2 rounded-full border border-slate-700 bg-slate-800/70 px-4 py-1.5">
              <span className="h-2 w-2 rounded-full bg-sky-400" />
              <span className="font-mono text-xs text-slate-400">Only .wasm files accepted</span>
            </div>
          </div>
        )}

        {(uploadState === 'scanning' || uploadState === 'submitting') && (
          <div className="flex w-full flex-col items-center gap-3 px-4 text-center">
            <WasmIcon state={uploadState} />
            <p className="text-base font-semibold tracking-wide text-violet-300">
              {uploadState === 'submitting' ? 'Validating with server...' : 'Scanning contract...'}
            </p>
            {droppedFile && (
              <div className="flex items-center gap-2 rounded-full border border-slate-700 bg-slate-800/70 px-3 py-1.5 font-mono text-xs text-slate-400">
                <span className="truncate max-w-[240px]">{droppedFile.name}</span>
                <span className="text-slate-500">.</span>
                <span>{formatBytes(droppedFile.sizeBytes)}</span>
              </div>
            )}
            <ScanningAnimation />
            <SpinnerDots />
            <p className="text-xs text-slate-500">
              {uploadState === 'submitting' ? 'Sending to backend for validation...' : 'Parsing WASM binary...'}
            </p>
          </div>
        )}

        {uploadState === 'success' && droppedFile && (
          <div className="flex flex-col items-center gap-4 text-center">
            <WasmIcon state="success" />
            <div>
              <p className="text-base font-semibold text-emerald-400">
                <SuccessIcon />
                Contract uploaded successfully
              </p>
              <p className="mt-1 text-xs text-slate-500">Ready for resource analysis</p>
            </div>
            <div className="flex items-center gap-3 rounded-xl border border-emerald-700/40 bg-slate-800/80 px-5 py-3">
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-emerald-700 bg-emerald-900/50">
                <span className="font-mono text-xs font-bold text-emerald-400">WA</span>
              </div>
              <div className="text-left">
                <p className="max-w-[220px] truncate text-sm font-medium text-slate-200">{droppedFile.name}</p>
                <p className="font-mono text-xs text-slate-500">{formatBytes(droppedFile.sizeBytes)}</p>
              </div>
            </div>
            <button
              type="button"
              id="wasm-upload-reset-btn"
              onClick={handleReset}
              className="mt-1 text-xs text-slate-500 underline underline-offset-2 transition-colors hover:text-slate-300"
            >
              Upload a different file
            </button>
          </div>
        )}

        {uploadState === 'error' && (
          <div className="flex flex-col items-center gap-4 text-center">
            <WasmIcon state="error" />
            <div>
              <p className="text-base font-semibold text-red-400">
                <ErrorIcon />
                {errorDetails?.title ?? 'File rejected'}
              </p>
              <p className="mt-2 max-w-[320px] text-xs leading-relaxed text-red-300/70">
                {errorDetails?.message}
              </p>
            </div>
            {errorDetails?.suggestedAction && (
              <div className="max-w-[320px] rounded-lg border border-amber-800/30 bg-amber-950/40 px-3 py-2">
                <span className="text-xs leading-relaxed text-amber-200/80">{errorDetails.suggestedAction}</span>
              </div>
            )}
            <button
              type="button"
              id="wasm-upload-try-again-btn"
              onClick={handleReset}
              className="mt-1 rounded-lg border border-slate-700 bg-slate-800 px-5 py-2 text-sm text-slate-300 transition-all hover:bg-slate-700 hover:text-white"
            >
              Try again
            </button>
          </div>
        )}
      </div>

      <p className="mt-3 text-center font-mono text-xs text-slate-600">
        WASM Resource Analyzer . Soroscope . compiled Soroban contracts only
      </p>

      <style>{`
        @keyframes pulse-ring {
          0%, 100% { opacity: 0.4; }
          50% { opacity: 1; }
        }
      `}</style>
    </div>
  );
}
