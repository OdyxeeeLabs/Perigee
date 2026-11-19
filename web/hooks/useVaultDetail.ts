"use client";

import { useEffect, useState } from "react";
import type { VaultDetail } from "@/types/vault";
import { fetchVaultDetail } from "@/lib/vault-detail";

export type VaultDetailState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: VaultDetail }
  | { status: "error"; error: string };

/**
 * Loads vault detail on demand — issue #684.
 *
 * Fetching only starts once `id` is non-null, so a collapsed modal/drawer
 * never pays the cost of the detail fetch; opening it (passing a real id)
 * is what triggers the request.
 */
export function useVaultDetail(id: string | null): VaultDetailState {
  const [state, setState] = useState<VaultDetailState>({ status: "idle" });

  useEffect(() => {
    if (!id) {
      setState({ status: "idle" });
      return;
    }

    let cancelled = false;
    setState({ status: "loading" });

    fetchVaultDetail(id)
      .then((data) => {
        if (!cancelled) setState({ status: "success", data });
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setState({
            status: "error",
            error: err instanceof Error ? err.message : "Failed to load vault detail",
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [id]);

  return state;
}
