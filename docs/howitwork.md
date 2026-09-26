How it works
1. The Vault (custody layer)
Each user deposits into their own Policy Vault — a Stellar multi-signature account paired with a Soroban policy contract that scopes exactly what the strategy agent is allowed to do:

✅ Swap basket asset A for basket asset B, within the strategy's approved asset list
✅ Add/remove liquidity in the approved rotation pool
✅ Claim and route the performance fee once a high-water mark is crossed
❌ Transfer funds to any address outside the vault
❌ Change the vault's own signers or policy rules
❌ Approve arbitrary contract calls
This mirrors the Safe + scoped-permissions model used in EVM portfolio tooling, rebuilt natively on Stellar's account model and Soroban, so the keeper has just enough rope to run the strategy and no more.

2. The Strategy Engine
A single, published, rules-based rotation:

Phase	Position	Trigger to exit
Bull	BTC/ETH basket (bridged/anchored representations held natively in the vault)	Drawdown or cycle-phase trigger fires
Bear	Auto-rebalanced stable LP (synthetic-stable / USDC pair) on a Stellar AMM	Cycle-phase trigger flips back to bull
The strategy stepping into the LP position during the bear is designed to sidestep the worst of the drawdown while still earning yield on idle capital, rather than sitting in cash or trying to time the bottom.

3. The Agent Fleet
Strategy execution isn't a single centralized bot — it's a fleet of autonomous agents, each with its own on-chain identity, deployed and orchestrated the same way you'd manage any fleet of workers:

Each agent is bound to one or more vaults it's authorized (by that vault's policy contract) to operate.
Agents carry an on-chain identity and reputation record — every rebalance/rotation call is attributable to a specific agent, and misbehaving or underperforming agents can be rotated out without touching user funds.
Fleet orchestration means the same infrastructure that runs your own vault also scales horizontally to run thousands of vaults for a white-label partner, with per-agent monitoring, health checks, and failover.
4. The Payment Rail
Two kinds of value move through Perigee, and both settle over the same private, tokenless payment layer:

Performance fees — computed on-chain against each vault's high-water mark, claimed by the protocol only on realized gains above that mark.
Agent micropayments — the machine-to-machine payments that keep the fleet running (data feeds, execution routing, cross-agent coordination), settled the same way a human's fee would be: non-custodially, without requiring anyone to hold a proprietary token.
No new token. No custodial fee wallet. Payments are private by default and visible only to the parties involved.
