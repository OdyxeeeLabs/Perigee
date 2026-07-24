"use client";

import React, { useEffect } from "react";
import type { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit";
import { createStore, createUseStore, shallow } from "../lib/createStore";

interface WalletState {
  connect: (moduleId: string) => Promise<void>;
  disconnect: () => Promise<void>;
  address: string | null;
  isConnecting: boolean;
  selectedWalletId: string | null;
  openModal: () => void;
  closeModal: () => void;
  isModalOpen: boolean;
  supportedWallets: { id: string; name: string; icon: string }[];
  error: string | null;
  kit: StellarWalletsKit | null;
}

const supportedWallets = [
  { id: "freighter", name: "Freighter", icon: "https://stellar.creit.tech/wallet-icons/freighter.png" },
  { id: "albedo", name: "Albedo", icon: "https://stellar.creit.tech/wallet-icons/albedo.png" },
  { id: "xbull", name: "xBull", icon: "https://stellar.creit.tech/wallet-icons/xbull.png" },
  { id: "rabet", name: "Rabet", icon: "https://stellar.creit.tech/wallet-icons/rabet.png" },
  { id: "lobstr", name: "Lobstr", icon: "https://stellar.creit.tech/wallet-icons/lobstr.png" },
];

const walletStore = createStore<WalletState>((set, get) => ({
  address: null,
  isConnecting: false,
  selectedWalletId: null,
  isModalOpen: false,
  error: null,
  kit: null,
  supportedWallets,

  connect: async (moduleId: string) => {
    const { kit } = get();
    if (!kit) {
      set({ error: "Wallet kit not loaded yet" });
      return;
    }

    set({ isConnecting: true, error: null });

    try {
      kit.setWallet(moduleId);
      const { address: walletAddress } = await kit.getAddress();

      set({
        address: walletAddress,
        selectedWalletId: moduleId,
        isModalOpen: false,
      });
      localStorage.setItem("inheritx_wallet_address", walletAddress);
      localStorage.setItem("inheritx_wallet_id", moduleId);
      sessionStorage.setItem("perigee_wallet_id", moduleId);
    } catch (err: unknown) {
      const errorMessage = err instanceof Error ? err.message : "Connection failed";
      set({ error: errorMessage });
      console.error("Wallet connection failed:", err);
    } finally {
      set({ isConnecting: false });
    }
  },

  disconnect: async () => {
    const { kit } = get();
    if (kit) {
      try {
        await kit.disconnect();
      } catch (err) {
        console.error("Disconnect error:", err);
      }
    }
    set({ address: null, selectedWalletId: null, error: null });
    localStorage.removeItem("inheritx_wallet_address");
    localStorage.removeItem("inheritx_wallet_id");
    sessionStorage.removeItem("perigee_wallet_id");
  },

  openModal: () => set({ error: null, isModalOpen: true }),
  closeModal: () => set({ error: null, isModalOpen: false }),
}));

let didInit = false;

async function initWalletKit() {
  if (didInit) return;
  didInit = true;

  try {
    const walletKitModule = await import("@creit.tech/stellar-wallets-kit");

    const savedAddress = localStorage.getItem("inheritx_wallet_address");
    const savedWalletId = localStorage.getItem("inheritx_wallet_id");

    const kitInstance = new walletKitModule.StellarWalletsKit({
      network: walletKitModule.WalletNetwork.TESTNET,
      selectedWalletId: savedWalletId || walletKitModule.FREIGHTER_ID,
      modules: walletKitModule.allowAllModules(),
    });

    walletStore.setState({ kit: kitInstance });

    if (savedAddress && savedWalletId) {
      try {
        kitInstance.setWallet(savedWalletId);
        const { address: walletAddress } = await kitInstance.getAddress();
        walletStore.setState({ address: walletAddress, selectedWalletId: savedWalletId });
        sessionStorage.setItem("perigee_wallet_id", savedWalletId);
      } catch (err) {
        console.error("Auto-reconnect failed:", err);
        localStorage.removeItem("inheritx_wallet_address");
        localStorage.removeItem("inheritx_wallet_id");
        sessionStorage.removeItem("perigee_wallet_id");
      }
    }
  } catch (err) {
    console.error("Failed to initialize wallet kit:", err);
    walletStore.setState({ error: "Failed to load wallet kit" });
  }
}

const useWalletStoreImpl = createUseStore(walletStore);

/** Select a slice of wallet state. Pass `shallow` as the equality fn when selecting an object/array. */
export function useWalletStore<U>(
  selector: (state: WalletState) => U,
  equalityFn?: (a: U, b: U) => boolean,
): U {
  return useWalletStoreImpl(selector, equalityFn);
}

const selectAll = (state: WalletState) => ({
  connect: state.connect,
  disconnect: state.disconnect,
  address: state.address,
  isConnected: !!state.address,
  isConnecting: state.isConnecting,
  selectedWalletId: state.selectedWalletId,
  openModal: state.openModal,
  closeModal: state.closeModal,
  isModalOpen: state.isModalOpen,
  supportedWallets: state.supportedWallets,
  error: state.error,
});

/**
 * Back-compat convenience hook returning the full wallet slice.
 * Prefer `useWalletStore(selector, shallow)` in new code to avoid re-rendering
 * on unrelated state changes (e.g. a component that only reads `address`
 * shouldn't re-render when `isModalOpen` toggles).
 */
export const useWallet = () => useWalletStoreImpl(selectAll, shallow);

export const WalletProvider = ({ children }: { children: React.ReactNode }) => {
  useEffect(() => {
    initWalletKit();
  }, []);

  return <>{children}</>;
};
