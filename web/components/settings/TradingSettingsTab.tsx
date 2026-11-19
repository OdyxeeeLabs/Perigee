"use client";

import { useLocale } from "@/components/LocaleSync";
import { FormattedPercent } from "@/components/ui/FormattedAmount";
import { useSettingsStore } from "@/lib/settingsStore";
import { GAS_PRICE_STRATEGIES, MAX_SLIPPAGE, MIN_SLIPPAGE } from "@/lib/settings";

/** Trading tab — issue #682: slippage, gas strategy, transaction deadline. */
export function TradingSettingsTab() {
  const trading = useSettingsStore((s) => s.settings.trading);
  const updateTrading = useSettingsStore((s) => s.updateTrading);
  const locale = useLocale();

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
            onChange={(e) => updateTrading({ slippageTolerance: Number(e.target.value) })}
            className="flex-1 accent-cyan-500"
            aria-describedby="slippage-value"
          />
          <span id="slippage-value" className="w-16 shrink-0 text-end font-mono text-sm text-cyan-400">
            <FormattedPercent value={trading.slippageTolerance} locale={locale} digits={1} />
          </span>
        </div>
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200" id="gas-strategy-label">
          Gas price strategy
        </span>
        <div className="mt-2 flex flex-wrap gap-2" role="group" aria-labelledby="gas-strategy-label">
          {GAS_PRICE_STRATEGIES.map((strategy) => (
            <button
              key={strategy}
              type="button"
              onClick={() => updateTrading({ gasPriceStrategy: strategy })}
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

        {trading.gasPriceStrategy === "custom" && (
          <div className="mt-3">
            <label htmlFor="custom-gas" className="text-xs text-slate-400">
              Custom gas price (stroops)
            </label>
            <input
              id="custom-gas"
              type="number"
              min={0}
              value={trading.customGasPrice}
              onChange={(e) => updateTrading({ customGasPrice: Number(e.target.value) })}
              className="mt-1 w-40 rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
            />
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
          id="tx-deadline"
          type="number"
          min={1}
          max={180}
          value={trading.transactionDeadlineMinutes}
          onChange={(e) => updateTrading({ transactionDeadlineMinutes: Number(e.target.value) })}
          className="mt-2 w-32 rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        />
      </section>
    </div>
  );
}
