# Strategy simulation (`POST /strategies/:id/simulate`)

Preview how an **approved** strategy would react to market inputs you supply,
before requesting execution. Implemented in `core/src/strategy_simulation.rs`
on top of the strategy trigger engine (`core/src/hysteresis.rs`).

A simulation is **read-only**. It runs the strategy's approved thresholds in a
fresh trigger evaluator and reads the vault's recorded position. It never
changes the live strategy state or the vault, never submits a transaction,
and two identical requests give identical answers.

## Request

```json
{
  "asset": "XLM",
  "reference_price_e7": 1200000,
  "market_data_as_of": "2026-11-19T08:29:00Z",
  "scenario": [
    { "label": "t+1h", "price_e7": 1140000 },
    { "label": "crash", "price_e7": 1020000 },
    { "label": "recovery", "price_e7": 1180000 }
  ]
}
```

| Field | Rules |
| --- | --- |
| `asset` | 1–12 alphanumeric characters. Must be the strategy's asset. |
| `reference_price_e7` | Price the drawdown is measured from, 7-decimal fixed point, 1 to 10^15. |
| `market_data_as_of` | RFC 3339 with offset. Not more than 5 minutes in the future and not older than 24 hours. |
| `scenario` | 1–100 steps applied in order. `price_e7` follows the same bounds as the reference price; `label` is optional, up to 64 printable characters. |

Unknown fields, including anything that looks like an execution flag, are
rejected with `400`. Validation failures return `422` with field-level
`errors`. Strategies that are not approved return `409`. Another manager's
strategy, or one outside a scoped token's vaults, returns `404`.

## Response

```json
{
  "kind": "simulation_estimate",
  "status": "simulated",
  "ledger_effect": "none",
  "disclaimer": "Estimate only. This is not an on-chain transaction, has not been submitted, and is not a financial guarantee.",
  "strategy_id": "s1",
  "vault_id": "vault-s1",
  "asset": "XLM",
  "assumptions": ["Prices are the caller-supplied scenario values, applied in order.", "..."],
  "data_timestamps": {
    "market_data_as_of": "2026-11-19T08:29:00Z",
    "position_as_of": "2026-11-19T08:25:00Z",
    "simulated_at": "2026-11-19T08:30:00Z"
  },
  "steps": [
    { "index": 1, "label": "crash", "price_e7": 1020000, "drawdown_bps": 1500,
      "trigger": "entered", "would_request_action": "rotate_to_stable",
      "strategy_state": "triggered", "estimated_value_stroops": "1020000000" }
  ],
  "summary": { "triggers_fired": 2, "final_strategy_state": "idle",
               "min_estimated_value_stroops": "1020000000", "max_estimated_value_stroops": "1180000000" }
}
```

### Estimates versus confirmed results

* `kind`, `status: "simulated"` and `ledger_effect: "none"` are always present.
  Confirmed ledger results come from the transaction endpoints, which carry a
  transaction hash and ledger sequence. A simulation never has either.
* `would_request_action` is what the strategy would *ask for*. Nothing is
  executed.
* `estimated_value_stroops` is position × scenario price. It ignores fees,
  slippage, liquidity and execution delay (see `assumptions`). Values are
  strings because they can exceed JSON's safe integer range.
* `data_timestamps` states how fresh each input was: your market data, the
  vault position, and when the simulation ran.
