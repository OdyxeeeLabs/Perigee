"use client";

import { useEffect, useState } from "react";
import { API_URL } from "../lib/api";
import { useRpcStatus } from "./useRpcStatus";
import {
  resolveTransactionGate,
  type TransactionGate,
} from "../lib/network/transactionGate";

export interface TransactionNetworkStatus extends TransactionGate {
  checking: boolean;
  lastHealthyAt: number | null;
  retry: () => void;
}

export function useTransactionNetwork(): TransactionNetworkStatus {
  const { health, checking, lastHealthyAt, retry } = useRpcStatus(API_URL);
  const [browserOnline, setBrowserOnline] = useState(true);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const update = () => setBrowserOnline(window.navigator.onLine);
    update();
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
    };
  }, []);

  const gate = resolveTransactionGate({
    online: browserOnline,
    reachable: health === "unknown" ? null : health === "healthy",
    lastHealthyAt,
  });

  return { ...gate, checking, lastHealthyAt, retry };
}
