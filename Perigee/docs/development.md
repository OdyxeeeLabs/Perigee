# Development Guide

This guide covers setting up your environment and the development standards we follow at Perigee.

## 🚀 Getting Started

### Prerequisites

- **Rust** (latest stable)
- **Node.js** (>= 18)
-  `Soroban CLI`

### Monorepo Setup

1.  **Fork** the repository and clone it locally.
2.  **Rust Core**: Build the backend.
    ```bash
    cargo build -p Perigee-core
    ```
3.  **Web Dashboard**: Install frontend dependencies.
    ```bash
    cd web
    npm install
    ```
4.  **Contracts**: Compile the sample contracts.
    ```bash
    cargo build --target wasm32-unknown-unknown --release
    ```

## 🔌 Development Proxy

The development proxy provides local backend mocking and simulation so frontend work can proceed without a running node or testnet deployment. It is zero-config for new developers: with no `dev.proxy.json` present, the proxy starts with sensible defaults and mocks the core API surface.

### Starting the Proxy

From the repository root:

```bash
bash scripts/dev-proxy.sh
```

Or, from the `web` directory:

```bash
npm run dev:proxy
```

The proxy listens on `http://localhost:8787` by default and forwards unmatched requests to the configured upstream. Point your frontend at the proxy base URL to activate mocking.

### Configuration: `dev.proxy.json`

Create a `dev.proxy.json` at the repository root to override defaults. All fields are optional.

```json
{
  "port": 8787,
  "upstream": "http://localhost:3000",
  "mocks": {
    "enabled": true,
    "dir": "./dev/mocks",
    "fallbackToUpstream": true
  },
  "contracts": {
    "enabled": false,
    "fork": {
      "chainId": 1,
      "rpcUrl": "http://localhost:8545",
      "blockNumber": "latest"
    },
    "anvm": {
      "binary": "anvm",
      "args": ["--fork-url", "http://localhost:8545", "--port", "8546"]
    }
  },
  "network": {
    "latencyMs": 0,
    "jitterMs": 0,
    "packetLoss": 0,
    "throttleKbs": 0
  },
  "logging": {
    "enabled": true,
    "dir": "./dev/logs",
    "redactHeaders": ["authorization", "cookie"]
  },
  "scenarios": {
    "dir": "./dev/scenarios",
    "active": null
  }
}
```

### Mock API Responses (MSW)

Mocks are defined as JSON files under the configured mock directory (default `./dev/mocks`). Each file declares a method, path, and response:

```json
{
  "method": "GET",
  "path": "/api/v1/analysis/:reportId",
  "status": 200,
  "headers": { "content-type": "application/json" },
  "body": {
    "id": ":reportId",
    "status": "complete",
    "findings": []
  }
}
```

Path parameters (e.g. `:reportId`) are substituted into the response body. When `fallbackToUpstream` is true, requests without a matching mock are forwarded to the upstream.

### Contract Simulation (Anvil / Hardhat)

Enable `contracts.enabled` to fork a live network with Anvil or Hardhat. The proxy starts the fork automatically and routes JSON-RPC requests to the forked node. Use `fork.blockNumber` to pin a specific block for deterministic tests.

Example with Anvil:

```bash
anvm --fork-url http://localhost:8545 --port 8546
```

The proxy exposes the forked node at `http://localhost:8546` and can be pointed at by the frontend contract client.

### Network Condition Simulation

Set `network.latencyMs` to add a fixed delay to every response. Add `network.jitterMs` for random variation. Set `network.packetLoss` (0 to 1) to drop a fraction of requests. Set `network.throttleKbp` to limit response bandwidth.

```json
{
  "network": {
    "latencyMs": 250,
    "jitterMs": 50,
    "packetLoss": 0.05,
    "throttleKbs": 512
  }
}
```

### Request / Response Logging

When `logging.enabled` is true, the proxy writes JSON-lines files to `logging.dir`. Each line contains timestamp, method, path, status, duration, and request/response bodies. Headers listed in `redactHeaders` are masked before writing.

Logs can be replayed as scenarios (see below) or inspected directly:

```bash
ls ./dev/logs
```

### Scenario Replay

A scenario is a saved sequence of requests and responses that can be replayed deterministically. Save logs into `scenarios.dir` and set `scenarios.active` to the file name to replay:

```json
{
  "scenarios": {
    "dir": "./dev/scenarios",
    "active": "checkout-failure.jsonl"
  }
}
```

While a scenario is active, the proxy returns the recorded responses in order, including the recorded network conditions.

## 🛠 Development Standards

### Rust Code

- **Formatting**: Always run `cargo fmt` before committing.
- **Linting**: Run `cargo clippy` to check for common mistakes.
- **Tests**: Ensure all tests pass with `cargo test`.

### Frontend (Next.js)

- **Styling**: Use Tailwind CSS for consistency.
- **Linting**: Run `npm run lint` within the `/web` directory.
- **Components**: Keep components modular and placed in `/web/components`.
- **State Persistence**: See [Frontend Persistence](./FRONTEND_PERSISTENCE.md) for details on how analysis results are persisted across page refreshes.

### Contracts

- Use **Soroban SGK v22.0.0** or higher.
- Avoid deprecated methods like `register_contract` (use `register` instead).
