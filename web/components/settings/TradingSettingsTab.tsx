"use client";

import { useSettingsStore } from "@/lib/settingsStore";
import { GAS_PRICE_STRATEGIES, MAX_SLIPPAGE, MIN_SLIPPAGE } from "@/lib/settings";
import { validateSettingsValues } from "@/lib/settingsValidation";
import { usePreferenceUndoToast } from "@/hooks/usePreferenceUndoToast";
import { useSettingsValidation } from "./SettingsValidationContext";

/** Trading tab — issue #682: slippage, gas strategy, transaction deadline. */
export function TradingSettingsTab() {
  const trading = useSettingsStore((s) => s.settings.trading);
  const settings = useSettingsStore((s) => s.settings);
  const updateTrading = useSettingsStore((s) => s.updateTrading);
  const { values, errors, setValue, setError, fieldProps } = useSettingsValidation();
  usePreferenceUndoToast("trading");

  function validateDraft(id: string, value: string) {
    if (!errors[id]) return;
    const message = validateSettingsValues({ ...values, [id]: value }, settings).find(
      (error) => error.id === id,
    );
    setError(id, message?.message ?? null);
  }

  function handleNumberField(id: string, value: string, patch: (parsed: number) => void) {
    setValue(id, value);
    validateDraft(id, value);
    if (value.trim() !== "" && Number.isFinite(Number(value))) patch(Number(value));
  }

  return (
    <div className="flex flex-col gap-6">
      <section>
        <label htmlFor="slippage" className="text-sm font-semibold text-slate-200">
          Slippage tolerance
        </label>
        <p className="mt-1 text-xs text-slate-400">
          Maximum acceptable price movement between quote and execution ({MIN_SLIPPAGE}%–{MAX_SLIPPAGE}%).
        </p>
        <div className="mt-2 flex max-w-md items-center gap-3">
          <input
            id="slippage"
            type="range"
            min={MIN_SLIPPAGE}
            max={MAX_SLIPPAGE}
            step={0.1}
            value={trading.slippageTolerance}
            onChange={(e) => {
              setValue("slippage", e.target.value);
              validateDraft("slippage", e.target.value);
              updateTrading({ slippageTolerance: Number(e.target.value) });
            }}
            className="flex-1 accent-cyan-500"
            aria-invalid={Boolean(errors.slippage)}
            aria-describedby={`slippage-value${errors.slippage ? " slippage-error" : ""}`}
          />
          <span id="slippage-value" className="w-16 shrink-0 text-right font-mono text-sm text-cyan-400">
            {trading.slippageTolerance.toFixed(1)}%
          </span>
        </div>
        {errors.slippage ? (
          <p id="slippage-error" className="mt-1 text-xs text-red-400">
            {errors.slippage}
          </p>
        ) : null}
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200" id="gas-strategy-label">
          Gas price strategy
        </span>
        <div
          id="gas-strategy"
          tabIndex={-1}
          role="group"
          aria-labelledby="gas-strategy-label"
          aria-invalid={Boolean(errors["gas-strategy"])}
          aria-describedby={errors["gas-strategy"] ? "gas-strategy-error" : undefined}
          className="mt-2 flex flex-wrap gap-2 focus:outline-none"
        >
          {GAS_PRICE_STRATEGIES.map((strategy) => (
            <button
              key={strategy}
              type="button"
              onClick={() => {
                setValue("gas-strategy", strategy);
                updateTrading({ gasPriceStrategy: strategy });
              }}
              aria-pressed={trading.gasPriceStrategy === strategy}
              className={`rounded px-3 py-1.5 text-sm font-medium capitalize transition-colors ${
                trading.gasPriceStrategy === strategy
                  ? "bg-cyan-500 text-white"
                  : "bg-slate-800 text-slate-300 hover:bg-slate-700"
              }`}
            >
              {strategy}
            </button>
          ))}
        </div>
        {errors["gas-strategy"] ? (
          <p id="gas-strategy-error" className="mt-1 text-xs text-red-400">
            {errors["gas-strategy"]}
          </p>
        ) : null}

        {trading.gasPriceStrategy === "custom" && (
          <div className="mt-3">
            <label htmlFor="custom-gas" className="text-xs text-slate-400">
              Custom gas price (stroops)
            </label>
            <input
              {...fieldProps("custom-gas")}
              type="number"
              min={0}
              value={values["custom-gas"] ?? String(trading.customGasPrice)}
              onChange={(e) =>
                handleNumberField("custom-gas", e.target.value, (parsed) =>
                  updateTrading({ customGasPrice: parsed }),
                )
              }
              className="mt-1 w-40 rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
            />
            {errors["custom-gas"] ? (
              <p id="custom-gas-error" className="mt-1 text-xs text-red-400">
                {errors["custom-gas"]}
              </p>
            ) : null}
          </div>
        )}
      </section>

      <section>
        <label htmlFor="tx-deadline" className="text-sm font-semibold text-slate-200">
          Transaction deadline (minutes)
        </label>
        <p className="mt-1 text-xs text-slate-400">
          How long an unconfirmed transaction remains valid before it is treated as expired.
        </p>
        <input
          {...fieldProps("tx-deadline")}
          type="number"
          min={1}
          max={180}
          value={values["tx-deadline"] ?? String(trading.transactionDeadlineMinutes)}
          onChange={(e) =>
            handleNumberField("tx-deadline", e.target.value, (parsed) =>
              updateTrading({ transactionDeadlineMinutes: parsed }),
            )
          }
          className="mt-2 w-32 rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        />
        {errors["tx-deadline"] ? (
          <p id="tx-deadline-error" className="mt-1 text-xs text-red-400">
            {errors["tx-deadline"]}
          </p>
        ) : null}
      </section>
    </div>
  );
}
