import { connectionState } from "../a11y/live-region";

export type TransactionNetworkStatus = ReturnType<typeof connectionState>;

export interface TransactionGateInput {
  online: boolean;
  reachable: boolean | null;
  lastHealthyAt?: number | null;
}

export interface TransactionGate {
  status: TransactionNetworkStatus;
  canTransact: boolean;
  disabledReason: string | null;
  showingCachedData: boolean;
}

export function resolveTransactionGate(input: TransactionGateInput): TransactionGate {
  const status = connectionState({ online: input.online, reachable: input.reachable });
  const canTransact = status === "online";

  return {
    status,
    canTransact,
    disabledReason:
      status === "offline"
        ? "Transactions are disabled while you are offline. Reconnect to submit."
        : status === "degraded"
          ? "Transactions are disabled because the chain data is unreachable. Reconnect to submit."
          : null,
    showingCachedData: !canTransact && Boolean(input.lastHealthyAt),
  };
}
