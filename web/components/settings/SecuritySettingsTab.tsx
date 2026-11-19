"use client";

import { useSettingsStore } from "@/lib/settingsStore";
import { SESSION_TIMEOUT_OPTIONS, type SessionTimeoutMinutes } from "@/lib/settings";
import { useWallet } from "@/context/WalletContext";
import { useSettingsValidation } from "./SettingsValidationContext";

const timeoutLabel = (minutes: SessionTimeoutMinutes) => (minutes === 0 ? "Never" : `${minutes} minutes`);

/**
 * Security tab — issue #682: 2FA, session timeout, connected wallets.
 *
 * Connected wallets reuse `useWallet()` from `context/WalletContext.tsx`
 * rather than re-reading `localStorage` wallet keys directly, so this tab
 * always reflects the same connection state as the rest of the app.
 */
export function SecuritySettingsTab() {
  const security = useSettingsStore((s) => s.settings.security);
  const updateSecurity = useSettingsStore((s) => s.updateSecurity);
  const { address, isConnected, selectedWalletId, supportedWallets, disconnect, openModal } = useWallet();
  const { values, errors, setValue, fieldProps } = useSettingsValidation();

  const walletLabel = supportedWallets.find((w) => w.id === selectedWalletId)?.name ?? selectedWalletId;

  return (
    <div className="flex flex-col gap-6">
      <section>
        <div className="flex items-center justify-between max-w-md">
          <div>
            <span className="text-sm font-semibold text-slate-200">Two-factor authentication</span>
            <p className="mt-1 text-xs text-slate-400">
              {security.twoFactorEnabled
                ? "Enabled — an additional verification step is required for sensitive actions."
                : "Not configured. Requires a backend enrollment flow (out of scope for this first cut)."}
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={security.twoFactorEnabled}
            aria-label="Toggle two-factor authentication"
            onClick={() => updateSecurity({ twoFactorEnabled: !security.twoFactorEnabled })}
            className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
              security.twoFactorEnabled ? "bg-cyan-500" : "bg-slate-700"
            }`}
          >
            <span
              className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                security.twoFactorEnabled ? "translate-x-5" : "translate-x-0.5"
              }`}
            />
          </button>
        </div>
      </section>

      <section>
        <label htmlFor="session-timeout" className="text-sm font-semibold text-slate-200">
          Session timeout
        </label>
        <select
          {...fieldProps("session-timeout")}
          value={values["session-timeout"] ?? String(security.sessionTimeoutMinutes)}
          onChange={(e) => {
            setValue("session-timeout", e.target.value);
            updateSecurity({ sessionTimeoutMinutes: Number(e.target.value) as SessionTimeoutMinutes });
          }}
          className="mt-2 w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {SESSION_TIMEOUT_OPTIONS.map((minutes) => (
            <option key={minutes} value={minutes}>
              {timeoutLabel(minutes)}
            </option>
          ))}
        </select>
        {errors["session-timeout"] ? (
          <p id="session-timeout-error" className="mt-1 text-xs text-red-400">
            {errors["session-timeout"]}
          </p>
        ) : null}
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200">Connected wallets</span>
        {isConnected ? (
          <div className="mt-2 flex max-w-md items-center justify-between rounded border border-slate-700 bg-slate-900 px-3 py-2">
            <div className="min-w-0">
              <div className="text-sm text-slate-200">{walletLabel}</div>
              <div className="truncate font-mono text-xs text-slate-400">{address}</div>
            </div>
            <button
              type="button"
              onClick={() => void disconnect()}
              className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs font-medium text-slate-300 hover:bg-slate-800"
            >
              Disconnect
            </button>
          </div>
        ) : (
          <div className="mt-2 flex max-w-md items-center justify-between rounded border border-slate-700 bg-slate-900 px-3 py-2">
            <span className="text-sm text-slate-400">No wallet connected</span>
            <button
              type="button"
              onClick={openModal}
              className="shrink-0 rounded bg-cyan-500 px-2.5 py-1 text-xs font-medium text-white hover:bg-cyan-600"
            >
              Connect
            </button>
          </div>
        )}
      </section>
    </div>
  );
}
