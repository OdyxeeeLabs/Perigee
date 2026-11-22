"use client";

import { useEffect, useRef, useState, type MouseEvent } from "react";
import { AlertCircle, Check, Copy } from "lucide-react";
import { Button } from "./Button";
import { LiveStatus } from "../LiveRegion";
import { copyValue, legacyCopyText, type CopyResult } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

export interface CopyButtonProps {
  /** The complete ID — never a shortened or truncated display value. */
  value: string;
  /** Noun used in the accessible name, e.g. "vault owner address". */
  label: string;
  className?: string;
  feedbackMs?: number;
}

export function CopyButton({ value, label, className, feedbackMs = 2500 }: CopyButtonProps) {
  const [result, setResult] = useState<CopyResult | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    },
    [],
  );

  const handleCopy = async (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    const writer = typeof navigator !== "undefined" ? navigator.clipboard : null;
    const outcome = await copyValue(value, writer, (text) => legacyCopyText(text, document));
    setResult(outcome);
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setResult(null), feedbackMs);
  };

  const statusMessage =
    result === "copied"
      ? `Copied full ${label}.`
      : result === "manual"
        ? `Copy failed. Select the full ${label} manually.`
        : "";

  return (
    <span className="inline-flex items-center gap-1.5">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        onClick={handleCopy}
        aria-label={result === "copied" ? `Copied full ${label}` : `Copy full ${label}`}
        title={`Copy full ${label}`}
        data-testid="copy-button"
        className={cn("h-6 w-6 shrink-0 text-slate-400 hover:text-cyan-300", className)}
      >
        {result === "copied" ? (
          <Check className="h-3.5 w-3.5 text-emerald-400" aria-hidden="true" />
        ) : result === "manual" ? (
          <AlertCircle className="h-3.5 w-3.5 text-amber-400" aria-hidden="true" />
        ) : (
          <Copy className="h-3.5 w-3.5" aria-hidden="true" />
        )}
      </Button>
      {result === "manual" && (
        <code data-testid="copy-fallback-value" className="select-all break-all font-mono text-xs text-amber-300">
          {value}
        </code>
      )}
      <LiveStatus message={statusMessage} data-testid="copy-status" />
    </span>
  );
}

export default CopyButton;
