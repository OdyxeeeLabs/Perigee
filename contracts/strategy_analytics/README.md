# Perigee Strategy Analytics

Risk framework and on-chain performance tracking for yield strategies.

Closes [#657](https://github.com/OdyxeeeLabs/Perigee/issues/657) and
[#658](https://github.com/OdyxeeeLabs/Perigee/issues/658).

## The problem this solves

A yield vault that always rotates into the highest headline APY is a vault that
eventually concentrates into whatever is paying the most right now — which is
usually whatever is riskiest. Two things are missing between "highest APY" and a
decision that is safe to make on user funds:

1. a **risk framework** that caps concentration before capital moves, and
2. **on-chain performance data** so a rotation is judged on realised return, not
   on a number read off a dashboard.

This contract provides both and holds no user funds itself, so it can be queried
by any vault or kept purely as an off-chain reference.

## Risk framework (#657)

Every registered strategy carries a `RiskProfile`:

| Field | Meaning |
| --- | --- |
| `risk_score` | 0–100, higher is riskier |
| `max_allocation_bps` | hard cap on this strategy's share of the book |
| `correlation_group` | strategies that fail together share a group id |

`RiskLimits` adds the portfolio-wide constraints:

| Field | Meaning |
| --- | --- |
| `max_portfolio_risk_score` | ceiling on the weighted-average risk score |
| `max_strategy_allocation_bps` | default per-strategy cap |
| `max_group_allocation_bps` | ceiling on exposure to one correlation group |
| `stress_tolerance_bps` | loss a stress scenario may produce before rejection |

### `validate_allocation`

Rejects, in order:

1. weights that do not sum to exactly 100%,
2. a position above its strategy's `max_allocation_bps`,
3. a correlation group above `max_group_allocation_bps`,
4. a weighted risk score above `max_portfolio_risk_score`.

The weighted score is `Σ weight_bps × risk_score / 10_000`, **rounded down**.
Flooring means the check can only under-state risk, never inflate it, so a book
that passes is genuinely inside its ceiling.

### Correlation is modelled as groups, not a matrix

Two strategies that draw down together are placed in the same group and the
group cap does the work. This is deliberately coarse: a dense pairwise matrix is
`O(n²)` state that has to be re-estimated constantly and can be gamed, whereas a
group cap is a single number a risk owner can reason about and audit.

### `stress_test`

Applies a caller-supplied shock to each strategy and returns

```text
loss_bps = Σ (weight_bps × shock_bps) / 10_000
```

alongside the configured tolerance and a `breached` flag. Running it before
deploying lets a book be checked against a scenario whose loss is already known
to be unacceptable.

## Performance tracking (#658)

Each strategy — and the benchmark — has a `Series` updated in place by
`record_snapshot` / `record_benchmark`:

- `first_*` and `last_*` anchor the APY computation, so storage is constant
  regardless of how many snapshots are taken;
- `high_water_mark` and `max_drawdown_bps` are updated incrementally, so the
  worst drawdown over the whole history is available without storing history.

`performance(strategy)` returns the annualised return, the benchmark's
annualised return over the same period, the **excess**, and the strategy's
current and maximum drawdown.

APY is a simple (non-compounded) annualisation of the interval return:

```text
apy_bps = (last_assets - first_assets) / first_assets × 10_000 × SECONDS_PER_YEAR / elapsed
```

Compounding would assume a reinvestment pattern the series does not describe.

### Timestamps come from the ledger

Snapshots are timestamped from `Env::ledger`, never from a caller argument. A
caller that can choose its own timestamps can manufacture any APY it likes.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called twice. |
| 3 | `Unauthorized` | Caller is not the admin. |
| 4 | `StrategyNotRegistered` | No risk profile for the strategy. |
| 5 | `StrategyAlreadyRegistered` | Profile already exists. |
| 6 | `InvalidRiskScore` | Score above `MAX_RISK_SCORE`. |
| 7 | `InvalidAllocation` | Empty book, bad weights, or a limit above 100%. |
| 8 | `AllocationExceedsLimit` | Position above its strategy cap. |
| 9 | `GroupExceedsLimit` | Correlation group above its cap. |
| 10 | `PortfolioRiskTooHigh` | Weighted risk above the ceiling. |
| 11 | `InvalidScenario` | Shock above 100%. |
| 12 | `TooManyStrategies` | More than `MAX_STRATEGIES`. |
| 13 | `NotEnoughSamples` | Fewer than two snapshots. |
| 14 | `InvalidAmount` | Non-positive asset value. |
| 15 | `DivisionByZero` | Two snapshots share a timestamp. |
| 16 | `MathOverflow` | An `i128` step overflowed. |

## Events

- `init()` → `admin`
- `limits()` → `(max_portfolio_risk_score, max_group_allocation_bps)`
- `strat_reg(strategy)` → `(risk_score, max_allocation_bps, correlation_group)`
- `strat_upd(strategy)` → `(risk_score, max_allocation_bps, correlation_group)`

## Tests

```bash
cargo test -p perigee-strategy-analytics
```

Covers risk-profile registration and updates, every rejection path of
`validate_allocation`, stress-test loss accounting and breach detection, APY
annualisation (including a losing strategy reporting a negative APY), benchmark
comparison, and high-water-mark drawdown tracking.
