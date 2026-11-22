White-label: Perigee for Wealth Managers
Everything above is also exposed as an API, so a small wealth manager can offer "Perigee inside" to their own clients without building any of this themselves:

Vault provisioning API — spin up a scoped Policy Vault per end-client, under the wealth manager's own branding.
Fleet-as-a-service — the wealth manager doesn't run their own agents; they rent capacity on the Perigee fleet, scoped per-client.
Reporting & reconciliation endpoints — performance, fee accrual, and high-water-mark status per client, exportable for the manager's own client reporting.
White-label fee splits — the manager sets their own client-facing fee on top of (or instead of) the base performance fee; settlement still runs through the same non-custodial rail.
