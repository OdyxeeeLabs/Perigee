Getting started
This repository contains the Soroban contracts (Policy Vault, Strategy Trigger, Fee Accrual), the agent fleet runtime, and the white-label API. See /contracts, /agent, and /api for component-level setup instructions.

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
