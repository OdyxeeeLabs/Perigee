'use client';

import React, { useCallback, useMemo, useState } from 'react';
import { FileRejection, useDropzone } from 'react-dropzone';
import { parseWasmError } from '../lib/errorHandling';

type UploadState = 'idle' | 'hover' | 'scanning' | 'submitting' | 'success' | 'error';

interface DroppedFile {
  name: string;
  sizeBytes: number;
}

interface ErrorDetails {
  title: string;
  message: string;
  details?: string;
  suggestedAction?: string;
}

export interface UploadZoneProps {
  onFileReady?: (file: File) => void;
  onReset?: () => void;
  backendUrl?: string;
  enableBackendValidation?: boolean;
}

const MAX_WASM_SIZE = 10 * 1024 * 1024;
const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function hasWasmMagic(buffer: ArrayBuffer): boolean {
  if (buffer.byteLength < 8) return false;
  const bytes = new Uint8Array(buffer, 0, 4);
  return WASM_MAGIC.every((byte, index) => bytes[index] === byte);
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

function WasmIcon({ state }: { state: UploadState }) {
  const stroke = {
    idle: '#64748b',
    hover: '#38bdf8',
    scanning: '#a78bfa',
    submitting: '#a78bfa',
    success: '#34d399',
    error: '#f87171',
  }[state];

  return (
    <svg width="64" height="64" viewBox="0 0 64 64" fill="none" className="transition-transform duration-300">
      <path
        d="M32 4 L56 18 L56 46 L32 60 L8 46 L8 18 Z"
        stroke={stroke}
        strokeWidth="2"
        fill="rgba(15,23,42,0.7)"
      />
      <text
        x="32"
        y="36"
        textAnchor="middle"
        fontSize="11"
        fontWeight="700"
        fontFamily="monospace"
        fill={stroke}
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
        style={{ animation: 'scan-sweep 1.6s ease-in-out infinite', backgroundSize: '200% 100%' }}
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

function StatusIcon({ state }: { state: 'success' | 'error' }) {
  return (
    <svg
      className={`mr-1.5 inline-block h-5 w-5 ${state === 'success' ? 'text-emerald-400' : 'text-red-400'}`}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2.5}
    >
      {state === 'success' ? (
        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
      ) : (
        <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
      )}
    </svg>
  );
}

export function UploadZone({
  onFileReady,
  onReset,
  backendUrl = 'http://localhost:8080/analyze/wasm',
  enableBackendValidation = true,
}: UploadZoneProps) {
  const [uploadState, setUploadState] = useState<UploadState>('idle');
  const [droppedFile, setDroppedFile] = useState<DroppedFile | null>(null);
  const [errorDetails, setErrorDetails] = useState<ErrorDetails | null>(null);

  const setError = useCallback((details: ErrorDetails) => {
    setErrorDetails(details);
    setUploadState('error');
    setDroppedFile(null);
  }, []);

  const submitToBackend = useCallback(
    async (file: File, buffer: ArrayBuffer) => {
      setUploadState('submitting');
      const response = await fetch(backendUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          wasm_bytes: arrayBufferToBase64(buffer),
          function_name: 'main',
          args: [],
        }),
      });

      if (!response.ok) {
        const raw = await response.text();
        let message = raw;
        try {
          const parsed = JSON.parse(raw);
          message = parsed.message || parsed.error || raw;
        } catch {
          message = raw;
        }
        setError(parseWasmError(response, message));
        return;
      }

      setUploadState('success');
      onFileReady?.(file);
    },
    [backendUrl, onFileReady, setError]
  );

  const onDropAccepted = useCallback(
    async (files: File[]) => {
      const file = files[0];
      setDroppedFile({ name: file.name, sizeBytes: file.size });
      setErrorDetails(null);
      setUploadState('scanning');

      try {
        const buffer = await file.arrayBuffer();
        if (!hasWasmMagic(buffer)) {
          setError({
            title: 'Invalid WASM File',
            message: `"${file.name}" is not a valid WebAssembly binary.`,
            suggestedAction: 'Upload a compiled Soroban .wasm contract.',
          });
          return;
        }

        if (enableBackendValidation) {
          await submitToBackend(file, buffer);
          return;
        }

        setUploadState('success');
        onFileReady?.(file);
      } catch (error) {
        setError({
          title: 'File Read Error',
          message: error instanceof Error ? error.message : 'Unable to read the selected file.',
          suggestedAction: 'Try selecting the file again.',
        });
      }
    },
    [enableBackendValidation, onFileReady, setError, submitToBackend]
  );

  const onDropRejected = useCallback(
    (rejections: FileRejection[]) => {
      const first = rejections[0];
      const fileName = first?.file?.name ?? 'file';
      const isTooLarge = first?.errors?.some((error) => error.code === 'file-too-large');
      const extension = fileName.includes('.') ? `.${fileName.split('.').pop()}` : 'unknown type';

      setError({
        title: isTooLarge ? 'File Too Large' : 'Invalid File Type',
        message: isTooLarge
          ? `"${fileName}" exceeds the ${MAX_WASM_SIZE / (1024 * 1024)} MB size limit.`
          : `"${fileName}" was rejected because only .wasm files are accepted (got ${extension}).`,
        suggestedAction: 'Please upload a compiled .wasm file.',
      });
    },
    [setError]
  );

  const validator = useCallback((file: File) => {
    if (!file.name.toLowerCase().endsWith('.wasm')) {
      return {
        code: 'file-invalid-type',
        message: `"${file.name}" is not a .wasm file.`,
      };
    }
    return null;
  }, []);

  const onDragEnter = useCallback(() => {
    if (uploadState !== 'scanning' && uploadState !== 'submitting') setUploadState('hover');
  }, [uploadState]);

  const onDragLeave = useCallback(() => {
    if (uploadState === 'hover') setUploadState('idle');
  }, [uploadState]);

  const { getRootProps, getInputProps, isDragActive, open } = useDropzone({
    onDropAccepted,
    onDropRejected,
    onDragEnter,
    onDragLeave,
    validator,
    accept: {
      'application/wasm': ['.wasm'],
      'application/octet-stream': ['.wasm'],
    },
    maxFiles: 1,
    maxSize: MAX_WASM_SIZE,
    noClick: uploadState === 'scanning' || uploadState === 'submitting',
    noDrag: uploadState === 'scanning' || uploadState === 'submitting',
  });

  const displayState = isDragActive && uploadState === 'idle' ? 'hover' : uploadState;
  const borderColor = useMemo(
    () =>
      ({
        idle: 'border-slate-600 hover:border-slate-400',
        hover: 'border-sky-400 shadow-[0_0_24px_rgba(56,189,248,0.2)]',
        scanning: 'border-violet-500 shadow-[0_0_24px_rgba(167,139,250,0.25)]',
        submitting: 'border-violet-500 shadow-[0_0_24px_rgba(167,139,250,0.25)]',
        success: 'border-emerald-500 shadow-[0_0_24px_rgba(52,211,153,0.2)]',
        error: 'border-red-500 shadow-[0_0_24px_rgba(248,113,113,0.2)]',
      })[displayState],
    [displayState]
  );

  const handleReset = (event: React.MouseEvent) => {
    event.stopPropagation();
    setUploadState('idle');
    setDroppedFile(null);
    setErrorDetails(null);
    onReset?.();
  };

  return (
    <div className="w-full font-sans">
      <div
        id="wasm-upload-zone"
        {...getRootProps()}
        className={[
          'relative flex min-h-[260px] cursor-pointer select-none flex-col items-center justify-center',
          'rounded-2xl border-2 border-dashed bg-slate-900/60 p-10',
          'transition-all duration-300 ease-in-out',
          borderColor,
        ].join(' ')}
        role="button"
        aria-label="WASM file upload zone"
      >
        <input {...getInputProps()} id="wasm-file-input" aria-label="Upload .wasm file" />

        {(displayState === 'idle' || displayState === 'hover') && (
          <div className="flex flex-col items-center gap-4 text-center">
            <WasmIcon state={displayState} />
            <div>
              <p className={`text-base font-semibold ${displayState === 'hover' ? 'text-sky-300' : 'text-slate-300'}`}>
                {displayState === 'hover' ? 'Release to upload your .wasm file' : 'Drag and drop your compiled .wasm file'}
              </p>
              <p className="mt-1 text-sm text-slate-500">
                or{' '}
                <button
                  type="button"
                  className="text-sky-400 underline underline-offset-2 hover:text-sky-300"
                  onClick={(event) => {
                    event.stopPropagation();
                    open();
                  }}
                >
                  click to browse
                </button>
              </p>
            </div>
            <span className="rounded-full border border-slate-700 bg-slate-800/70 px-4 py-1.5 font-mono text-xs text-slate-400">
              Only .wasm files accepted
            </span>
          </div>
        )}

        {(displayState === 'scanning' || displayState === 'submitting') && (
          <div className="flex w-full flex-col items-center gap-3 px-4 text-center">
            <WasmIcon state={displayState} />
            <p className="text-base font-semibold text-violet-300">
              {displayState === 'submitting' ? 'Validating with server...' : 'Scanning contract...'}
            </p>
            {droppedFile && (
              <div className="flex items-center gap-2 rounded-full border border-slate-700 bg-slate-800/70 px-3 py-1.5 font-mono text-xs text-slate-400">
                <span className="max-w-[240px] truncate">{droppedFile.name}</span>
                <span className="text-slate-500">-</span>
                <span>{formatBytes(droppedFile.sizeBytes)}</span>
              </div>
            )}
            <ScanningAnimation />
          </div>
        )}

        {displayState === 'success' && droppedFile && (
          <div className="flex flex-col items-center gap-4 text-center">
            <WasmIcon state="success" />
            <p className="text-base font-semibold text-emerald-400">
              <StatusIcon state="success" />
              Contract uploaded successfully
            </p>
            <div className="rounded-xl border border-emerald-700/40 bg-slate-800/80 px-5 py-3 text-left">
              <p className="max-w-[260px] truncate text-sm font-medium text-slate-200">{droppedFile.name}</p>
              <p className="font-mono text-xs text-slate-500">{formatBytes(droppedFile.sizeBytes)}</p>
            </div>
            <button
              type="button"
              id="wasm-upload-reset-btn"
              onClick={handleReset}
              className="text-xs text-slate-500 underline underline-offset-2 hover:text-slate-300"
            >
              Upload a different file
            </button>
          </div>
        )}

        {displayState === 'error' && (
          <div className="flex flex-col items-center gap-4 text-center">
            <WasmIcon state="error" />
            <div>
              <p className="text-base font-semibold text-red-400">
                <StatusIcon state="error" />
                {errorDetails?.title || 'File rejected'}
              </p>
              <p className="mt-2 max-w-[320px] text-xs leading-relaxed text-red-300/80">
                {errorDetails?.message || 'The selected file could not be uploaded.'}
              </p>
              {errorDetails?.details && (
                <p className="mt-3 max-w-[320px] rounded-lg border border-red-800/30 bg-red-950/40 p-3 text-left font-mono text-xs leading-relaxed text-red-200/80">
                  {errorDetails.details}
                </p>
              )}
            </div>
            {errorDetails?.suggestedAction && (
              <p className="max-w-[320px] rounded-lg border border-amber-800/30 bg-amber-950/40 px-3 py-2 text-xs leading-relaxed text-amber-200/80">
                {errorDetails.suggestedAction}
              </p>
            )}
            <button
              type="button"
              id="wasm-upload-try-again-btn"
              onClick={handleReset}
              className="rounded-lg border border-slate-700 bg-slate-800 px-5 py-2 text-sm text-slate-300 hover:bg-slate-700 hover:text-white"
            >
              Try again
            </button>
          </div>
        )}
      </div>

      <p className="mt-3 text-center font-mono text-xs text-slate-600">
        WASM Resource Analyzer - Soroscope - compiled Soroban contracts only
      </p>
    </div>
  );
}
