# Fee Switch (#666)

Governance-controlled fee activation, revenue sharing, and buyback/burn.

- `activate` / `deactivate` — governance gated
- `collect` — split revenue into treasury / stakers / buyback buckets (bps sum to 10_000)
- `execute_buyback` — spend buyback budget and record burns
- `quote_fee` — fee on notional when active (0 when inactive)
