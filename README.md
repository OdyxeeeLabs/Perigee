# Perigee

**Autonomous portfolio agents for Stellar — non-custodial, rules-based, and paid only on performance.**

Perigee is a non-custodial portfolio protocol on Stellar that runs a simple, rules-based macro strategy for you — hold a BTC/ETH basket through the bull market, rotate into an auto-rebalanced stable liquidity position during the bear — executed by a fleet of autonomous on-chain agents, settled through a private, tokenless micropayment rail, and offered white-label to wealth managers who want to run their own fleet under their own brand.

You never give up your keys. Perigee never touches your withdrawal rights. We only get paid when you make money.

---

## Why Perigee

Most "automated" crypto portfolio tools ask you to trust a custodian, a centralized bot operator, or an opaque fee structure. Perigee is built to remove all three:

- **Non-custodial by construction.** Funds live in your own smart wallet on Stellar. The strategy agent can rebalance and rotate positions — it can never move funds to an address outside the vault's policy.
- **Rules-based, not discretionary.** The strategy is a fixed, published set of triggers (halving-cycle phase, drawdown thresholds, volatility bands). No human is making ad-hoc calls with your money.
- **Agents, not a black box.** Every action a strategy agent takes is an on-chain call from an identifiable, reputation-tracked agent — inspectable, attributable, and revocable.
- **Aligned fees.** 10% performance fee above a high-water mark. No management fee, no fee if you're underwater, no fee on capital you put in and immediately withdraw.
- **Private settlement.** Fee payments and agent-to-agent micropayments settle over a shielded, tokenless payment rail — no new token to buy, no public ledger trail of exactly what you paid and when.

---

## How it works

### 1. The Vault (custody layer)

Each user deposits into their own **Policy Vault** — a Stellar multi-signature account paired with a Soroban policy contract that scopes exactly what the strategy agent is allowed to do:

- ✅ Swap basket asset A for basket asset B, within the strategy's approved asset list
- ✅ Add/remove liquidity in the approved rotation pool
- ✅ Claim and route the performance fee once a high-water mark is crossed
- ❌ Transfer funds to any address outside the vault
- ❌ Change the vault's own signers or policy rules
- ❌ Approve arbitrary contract calls

This mirrors the Safe + scoped-permissions model used in EVM portfolio tooling, rebuilt natively on Stellar's account model and Soroban, so the keeper has *just enough* rope to run the strategy and no more.

### 2. The Strategy Engine

A single, published, rules-based rotation:

| Phase | Position | Trigger to exit |
|---|---|---|
| **Bull** | BTC/ETH basket (bridged/anchored representations held natively in the vault) | Drawdown or cycle-phase trigger fires |
| **Bear** | Auto-rebalanced stable LP (synthetic-stable / USDC pair) on a Stellar AMM | Cycle-phase trigger flips back to bull |

The strategy stepping into the LP position during the bear is designed to sidestep the worst of the drawdown while still earning yield on idle capital, rather than sitting in cash or trying to time the bottom.

### 3. The Agent Fleet

Strategy execution isn't a single centralized bot — it's a **fleet of autonomous agents**, each with its own on-chain identity, deployed and orchestrated the same way you'd manage any fleet of workers:

- Each agent is bound to one or more vaults it's authorized (by that vault's policy contract) to operate.
- Agents carry an on-chain identity and reputation record — every rebalance/rotation call is attributable to a specific agent, and misbehaving or underperforming agents can be rotated out without touching user funds.
- Fleet orchestration means the same infrastructure that runs your own vault also scales horizontally to run thousands of vaults for a white-label partner, with per-agent monitoring, health checks, and failover.

### 4. The Payment Rail

Two kinds of value move through Perigee, and both settle over the same private, tokenless payment layer:

- **Performance fees** — computed on-chain against each vault's high-water mark, claimed by the protocol only on realized gains above that mark.
- **Agent micropayments** — the machine-to-machine payments that keep the fleet running (data feeds, execution routing, cross-agent coordination), settled the same way a human's fee would be: non-custodially, without requiring anyone to hold a proprietary token.

No new token. No custodial fee wallet. Payments are private by default and visible only to the parties involved.

---

## White-label: Perigee for Wealth Managers

Everything above is also exposed as an API, so a small wealth manager can offer "Perigee inside" to their own clients without building any of this themselves:

- **Vault provisioning API** — spin up a scoped Policy Vault per end-client, under the wealth manager's own branding.
- **Fleet-as-a-service** — the wealth manager doesn't run their own agents; they rent capacity on the Perigee fleet, scoped per-client.
- **Reporting & reconciliation endpoints** — performance, fee accrual, and high-water-mark status per client, exportable for the manager's own client reporting.
- **White-label fee splits** — the manager sets their own client-facing fee on top of (or instead of) the base performance fee; settlement still runs through the same non-custodial rail.

---

## Architecture

```
┌─────────────────────┐        ┌──────────────────────────┐
│   User / Client      │        │  Wealth Manager (WL API) │
│  (owns the vault)     │        │  provisions vaults for    │
└──────────┬────────────┘        │  many end-clients         │
           │                     └───────────┬──────────────┘
           ▼                                 ▼
┌────────────────────────────────────────────────────────────┐
│                       Policy Vault (Soroban)                 │
│   Stellar multi-sig account + scoped policy contract          │
│   allows: rebalance / rotate / claim fee                      │
│   forbids: arbitrary transfer, signer change, approvals        │
└──────────┬───────────────────────────────────────────────────┘
           │  scoped calls only
           ▼
┌────────────────────────────────────────────────────────────┐
│                     Strategy Agent Fleet                     │
│  on-chain identity · reputation · health checks · failover    │
│  reads: cycle-phase / drawdown / volatility triggers           │
│  executes: basket rotation via Stellar AMM (Soroswap-style)    │
└──────────┬───────────────────────────────────────────────────┘
           │  fee + micropayment settlement
           ▼
┌────────────────────────────────────────────────────────────┐
│                Private Payment Rail (tokenless)               │
│   performance fee settlement · agent-to-agent micropayments   │
└────────────────────────────────────────────────────────────┘
```

---

## Fee model

- **10% performance fee**, charged only above a per-vault high-water mark.
- **0% management fee.**
- Fees accrue and settle on-chain, computed from the vault's own realized NAV — never estimated or self-reported by the operator.
- White-label partners set their own client-facing markup independently of the base protocol fee.

---

## Non-custodial guarantees

- The protocol operator **cannot** withdraw user funds under any circumstance — the Policy Vault contract has no code path that allows it.
- The strategy agent's permissions are scoped to a fixed allow-list of contract calls, enforced on-chain, not by convention.
- Users can revoke agent authorization on their vault at any time and retain full sole custody via their own Stellar account signers.

---

## Tech stack

- **Stellar** — base settlement layer, native multi-signature accounts
- **Soroban** — policy vault contracts, strategy trigger logic, fee accrual/high-water-mark tracking
- **Stellar AMM (Soroswap-style pools)** — basket rotation and the stable LP rotation position
- **Anchored/bridged BTC & ETH representations** — for the bull-phase basket
- **Private tokenless payment rail** — fee settlement and agent-to-agent micropayments
- **Fleet orchestration layer** — agent identity, reputation, health/failover, per-vault authorization

---

## Getting started

> This repository contains the Soroban contracts (Policy Vault, Strategy Trigger, Fee Accrual), the agent fleet runtime, and the white-label API. See `/contracts`, `/agent`, and `/api` for component-level setup instructions.

```bash
# clone
git clone https://github.com/<org>/perigee.git
cd perigee

# install
npm install

# build & test the Soroban contracts
cd contracts && soroban contract build && cargo test

# run the agent runtime locally against Stellar testnet
cd ../agent && npm run start:testnet

# run the white-label API
cd ../api && npm run dev
```

---

## Roadmap

- [ ] Mainnet Policy Vault contract audit
- [ ] Additional rotation strategies (beyond BTC/ETH ↔ stable LP)
- [ ] Public agent reputation dashboard
- [ ] Expanded white-label reporting (tax-lot level detail)
- [ ] Multi-asset high-water-mark accounting (beyond single stable-denominated NAV)

---

## License

MIT
