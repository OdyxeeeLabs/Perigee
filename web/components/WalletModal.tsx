"use client";
import Image from "next/image";

import { useWalletStore } from "../context/WalletContext";
import { motion, AnimatePresence, useReducedMotion } from "framer-motion";
import { Wallet, Check, AlertCircle } from "lucide-react";
import React from "react";
import UserIcon from "./userIcon";
import { logger } from "../lib/logger";
import { trackTelemetryEvent } from "../lib/telemetry";
import { useTranslations } from "next-intl";

export function WalletModal() {
  const t = useTranslations("walletModal");
  const isModalOpen = useWalletStore((s) => s.isModalOpen);
  const closeModal = useWalletStore((s) => s.closeModal);
  const supportedWallets = useWalletStore((s) => s.supportedWallets);
  const connect = useWalletStore((s) => s.connect);
  const isConnecting = useWalletStore((s) => s.isConnecting);
  const error = useWalletStore((s) => s.error);
  const shouldReduceMotion = useReducedMotion();

  const [activeSelection, setActiveSelection] = React.useState<string | null>(
    null,
  );

  // Reset selection when modal opens
  React.useEffect(() => {
    if (isModalOpen) setActiveSelection(null);
  }, [isModalOpen]);

  const handleConnectClick = async () => {
    if (activeSelection) {
      try {
        await connect(activeSelection);
        trackTelemetryEvent({
          name: "wallet_connect",
          properties: { wallet_id: activeSelection },
        });
      } catch (err) {
        // Error is handled in context
        logger.error("Connection error:", err);
      }
    }
  };

  return (
    <AnimatePresence>
      {isModalOpen && (
        <>
          {/* Backdrop */}
          <div
            className="fixed inset-0 z-40 bg-black/50 backdrop-blur-sm"
            onClick={closeModal}
          />
          <motion.div
            initial={shouldReduceMotion ? { opacity: 1, scale: 1 } : { opacity: 0, scale: 0.95 }}
            animate={shouldReduceMotion ? { opacity: 1, scale: 1 } : { opacity: 1, scale: 1 }}
            exit={shouldReduceMotion ? { opacity: 1, scale: 1 } : { opacity: 0, scale: 0.95 }}
            transition={{ duration: shouldReduceMotion ? 0 : 0.2, ease: "easeOut" }}
            className="fixed inset-0 z-50 flex items-center justify-center p-4"
          >
            {/* Modal card — centred, responsive width */}
            <div className="w-96 max-w-[calc(100vw-2rem)] rounded-2xl bg-[#161E22] border border-[#2A3338] p-8 shadow-2xl">
            <div className="flex flex-col items-center">
              <div className="text-center mb-6">
                <h2 className="text-2xl font-medium text-white">
                  {t("title")}
                </h2>
                <p className="mt-2 text-[#92A5A8] text-sm">
                  {t("description")}
                </p>
              </div>

              {error && (
                <div className="w-full mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg flex items-center gap-2">
                  <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
                  <span className="text-red-400 text-sm">{error}</span>
                </div>
              )}

              <div className="flex flex-col gap-3 w-full mb-6">
                {supportedWallets.map((wallet) => {
                  const isSelected = activeSelection === wallet.id;

                  return (
                    <button
                      key={wallet.id}
                      onClick={() => setActiveSelection(wallet.id)}
                      className={`flex items-center gap-4 w-full p-4 rounded-xl transition-all border ${
                        isSelected
                          ? "bg-[#1a2333] border-[#33C5E0]/30"
                          : "bg-transparent border-[#2A3338] hover:bg-[#1a2333] hover:border-[#33C5E0]/20"
                      }`}
                    >
                      {/* Radio Circle */}
                      <div
                        className={`w-5 h-5 rounded-full border-2 flex items-center justify-center transition-colors ${
                          isSelected
                            ? "border-[#33C5E0] bg-[#33C5E0]"
                            : "border-[#2d3b4f]"
                        }`}
                      >
                        {isSelected && (
                          <Check
                            className="w-3 h-3 text-black"
                            strokeWidth={3}
                          />
                        )}
                      </div>

                      {/* Icon */}
                      <div className="text-[#92A5A8]">
                        <Image
                          src={wallet.icon}
                          alt={wallet.name}
                          width={20}
                          height={20}
                          className="object-contain"
                        />
                      </div>

                      <span className="font-medium text-white text-left flex-1">
                        {wallet.name}
                      </span>
                    </button>
                  );
                })}
              </div>

              <button
                onClick={handleConnectClick}
                disabled={!activeSelection || isConnecting}
                className={`w-full py-3 rounded-xl font-medium transition-all flex items-center justify-center gap-2 ${
                  activeSelection && !isConnecting
                    ? "bg-[#33C5E0] hover:bg-[#33C5E0]/90 text-black"
                    : "bg-[#2A3338] cursor-not-allowed text-gray-500"
                }`}
              >
                <UserIcon />
                <span>{isConnecting ? t("connecting") : t("connectButton")}</span>
              </button>
            </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
