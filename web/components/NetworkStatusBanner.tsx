"use client";

import React, { useEffect, useState, useCallback } from "react";
import { useTranslations } from "next-intl";
import { supportLinks } from "../lib/config";
import { getStage } from "../lib/contracts.config";

type NetworkStatus = "online" | "offline" | "api-down";

const API_CHECK_INTERVAL_MS = 20_000;
const API_TIMEOUT_MS = 4_000;

async function pingApi(apiUrl: string): Promise<boolean> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), API_TIMEOUT_MS);
    const res = await fetch(`${apiUrl}/health`, {
      method: "HEAD",
      signal: controller.signal,
      cache: "no-store",
    });
    clearTimeout(timer);
    return res.ok;
  } catch {
    return false;
  }
}

const messages: Record<Exclude<NetworkStatus, "online">, { title: string; body: string }> = {
  offline: {
    title: "network.offlineTitle",
    body: "network.offlineBody",
  },
  "api-down": {
    title: "network.apiDownTitle",
    body: "network.apiDownBody",
  },
};

interface NetworkStatusBannerProps {
  apiUrl: string;
  /** Show reconnect guidance when an offline transaction was blocked. */
  reconnectHint?: boolean;
}

export function NetworkStatusBanner({ apiUrl, reconnectHint = false }: NetworkStatusBannerProps) {
  const t = useTranslations();
  const [status, setStatus] = useState<NetworkStatus>("online");
  const [retryCount, setRetryCount] = useState(0);

  const stage = getStage();
  const isProduction = process.env.NODE_ENV === "production";
  const isTestnet = stage === "testnet";

  const networkLabel =
    stage === "mainnet"
      ? t("network.mainnetLabel")
      : stage === "testnet"
        ? t("network.testnetLabel")
        : t("network.localLabel");

  const networkDotColor =
    stage === "mainnet"
      ? "bg-emerald-400"
      : stage === "testnet"
        ? "bg-amber-400"
        : "bg-slate-400";

  const networkTextColor =
    stage === "mainnet"
      ? "text-emerald-300"
      : stage === "testnet"
        ? "text-amber-300"
        : "text-slate-400";

  const checkStatus = useCallback(async () => {
    if (!navigator.onLine) {
      setStatus("offline");
      return;
    }
    const up = await pingApi(apiUrl);
    setStatus(up ? "online" : "api-down");
  }, [apiUrl]);

  useEffect(() => {
    const goOnline = () => {
      setStatus("online");
      checkStatus();
    };
    const goOffline = () => setStatus("offline");

    window.addEventListener("online", goOnline);
    window.addEventListener("offline", goOffline);
    return () => {
      window.removeEventListener("online", goOnline);
      window.removeEventListener("offline", goOffline);
    };
  }, [checkStatus]);

  useEffect(() => {
    checkStatus();
    const interval = setInterval(checkStatus, API_CHECK_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [checkStatus, retryCount]);

  const handleRetry = useCallback(() => {
    setRetryCount((c) => c + 1);
  }, []);

  if (isTestnet && isProduction) {
    return (
      <div
        role="alert"
        aria-live="assertive"
        aria-atomic="true"
        className="flex items-start gap-3 px-4 py-3 text-sm bg-red-900/80 border-b border-red-700 text-red-100"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
          className="h-5 w-5 shrink-0 mt-0.5 text-red-400"
          aria-hidden="true"
        >
          <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
        <div className="flex-1">
          <span className="font-semibold">{t("network.testnetWarningTitle")}</span>{" "}
          <span className="opacity-90">{t("network.testnetWarningBody")}</span>
        </div>
      </div>
    );
  }

  if (status !== "online") {
    const { title, body } = messages[status];
    const isOffline = status === "offline";

    const displayTitle = t(title);
    const displayBody = t(body);

    return (
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className={[
          "flex items-start gap-3 px-4 py-3 text-sm",
          isOffline
            ? "bg-slate-800 border-b border-slate-700 text-slate-200"
            : "bg-amber-900/80 border-b border-amber-700 text-amber-100",
        ].join(" ")}
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
          className={[
            "h-5 w-5 shrink-0 mt-0.5",
            isOffline ? "text-slate-400" : "text-amber-400",
          ].join(" ")}
          aria-hidden="true"
        >
          {isOffline ? (
            <>
              <line x1="1" y1="1" x2="23" y2="23" />
              <path d="M16.72 11.06A10.94 10.94 0 0119 12.55" />
              <path d="M5 12.55a10.94 10.94 0 015.17-2.39" />
              <path d="M10.71 5.05A16 16 0 0122.56 9" />
              <path d="M1.42 9a15.91 15.91 0 014.7-2.88" />
              <path d="M8.53 16.11a6 6 0 016.95 0" />
              <line x1="12" y1="20" x2="12.01" y2="20" />
            </>
          ) : (
            <>
              <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </>
          )}
        </svg>

        <div className="flex-1">
          <span className="font-semibold">{displayTitle}</span>{" "}
          <span className="opacity-80">{displayBody}</span>{" "}
          {reconnectHint && isOffline && (
            <span className="font-semibold">Reconnect to complete your transaction.</span>
          )}{" "}
          {!isOffline && (
            <a
              href={supportLinks.supportUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="underline underline-offset-2 hover:opacity-100 opacity-90"
            >
              {t("network.getHelp")}
            </a>
          )}
        </div>

        {!isOffline && (
          <button
            type="button"
            onClick={handleRetry}
            className="shrink-0 rounded border border-amber-600 px-2.5 py-1 text-xs font-medium
                       text-amber-200 hover:bg-amber-800 focus:outline-none focus:ring-2
                       focus:ring-amber-400 disabled:opacity-50 transition-colors"
          >
            {t("network.retry")}
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between px-4 py-2 text-sm bg-slate-900 border-b border-slate-800 text-slate-300">
      <div className="flex items-center gap-2">
        <span
          className={["h-2 w-2 rounded-full", networkDotColor].join(" ")}
          aria-hidden="true"
        />
        <span className={networkTextColor}>{networkLabel}</span>
      </div>
      <span className="text-xs opacity-70">{t("network.onlineLabel")}</span>
    </div>
  );
}
