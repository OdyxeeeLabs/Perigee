# Feature Flags System

Runtime feature flag system for the Perigee web application.
Implements [FE-012](https://github.com/OdyxeeeLabs/Perigee/issues/219).

## Overview

The feature flag module lives in `web/features/feature-flags/` and provides:

- A `FeatureFlag` union type derived from the constants registry
- A `useFeatureFlag(flag)` React hook for conditional rendering
- A `useFeatureFlags()` hook for bulk access and refresh
- Env-var-based flag overrides via `NEXT_PUBLIC_FEATURE_FLAG_*`
- API-based remote flag sources with automatic polling

## Backend flags

The Rust backend uses the typed `config::FeatureFlagService`. Flags are read from `FEATURE_FLAGS` as a JSON object, JSON array of enabled names, or comma-separated `name=true|false` values. `ENABLE_VAULT_V2` and `ENABLE_NEW_FEE_MODEL` can override the aggregate value. Unknown names, malformed JSON, and non-boolean values stop startup.

| Backend flag | Default | Effect when enabled |
|---|---|---|
| `enable_vault_v2` | `false` | Exposes the authenticated `/v2/vaults` aliases |
| `enable_new_fee_model` | `false` | Exposes `/fees/v2/recommend` |

Disabled routes return the same generic not-found response as an unknown route. The web and backend flag namespaces are independent; do not use `NEXT_PUBLIC_*` values to control backend authorization or secret handling.

## File Structure

```
web/features/feature-flags/
  constants.ts            # Flag keys and defaults
  types.ts                # TypeScript types
  feature-flags.ts        # Flag definitions array
  featureFlagService.ts   # Singleton service (env + API sources)
  FeatureFlagContext.tsx   # React context + provider
  useFeatureFlag.ts       # React hooks
  index.ts                # Barrel re-exports
  featureFlagService.test.cjs  # Unit tests
```

## Usage

### 1. Check a single flag in a component

```tsx
import { useFeatureFlag } from "@/features/feature-flags";

export function VaultView() {
  const isNewUIEnabled = useFeatureFlag("newVaultUI");

  if (isNewUIEnabled) {
    return <NewVaultDashboard />;
  }
  return <LegacyVaultDashboard />;
}
```

### 2. Access all flags and refresh from API

```tsx
import { useFeatureFlags } from "@/features/feature-flags";

export function FlagDebugPanel() {
  const { flags, refresh } = useFeatureFlags();

  return (
    <div>
      {Object.entries(flags).map(([key, enabled]) => (
        <div key={key}>
          {key}: {enabled ? "ON" : "OFF"}
        </div>
      ))}
      <button onClick={refresh}>Refresh from API</button>
    </div>
  );
}
```

### 3. Programmatic toggle (non-React code)

```ts
import { featureFlagService } from "@/features/feature-flags";

featureFlagService.enable("dashboardV2");
featureFlagService.toggle("experimentalCharts");
featureFlagService.disable("notificationsV2");
```

## Adding a New Flag

1. Add the flag key to `FEATURE_FLAGS` in `constants.ts`:

```ts
export const FEATURE_FLAGS = {
  // ...existing flags
  NEW_CHECKOUT_FLOW: "newCheckoutFlow",
} as const;
```

2. Add the definition to the `featureFlags` array in `feature-flags.ts`:

```ts
{
  key: FEATURE_FLAGS.NEW_CHECKOUT_FLOW,
  enabled: false,
  description: "Enable the new checkout experience.",
}
```

3. Add the env-var override to `.env.example`:

```env
NEXT_PUBLIC_FEATURE_FLAG_newCheckoutFlow=false
```

The `FeatureFlag` type is automatically derived from the constants, so no manual type updates are needed.

## Flag Sources (Priority Order)

Flags are resolved in the following order (last wins):

1. **Default values** — defined in `feature-flags.ts`
2. **Environment variables** — `NEXT_PUBLIC_FEATURE_FLAG_{flagKey}`
3. **API source** — optional remote endpoint returning `{ flagKey: boolean }` JSON

## Environment Variables

| Variable | Type | Default | Description |
|---|---|---|---|
| `NEXT_PUBLIC_FEATURE_FLAG_newVaultUI` | `string` | `"false"` | Toggle new vault UI |
| `NEXT_PUBLIC_FEATURE_FLAG_dashboardV2` | `string` | `"false"` | Toggle dashboard v2 |
| `NEXT_PUBLIC_FEATURE_FLAG_experimentalCharts` | `string` | `"true"` | Toggle experimental charts |
| `NEXT_PUBLIC_FEATURE_FLAG_notificationsV2` | `string` | `"false"` | Toggle notifications v2 |

Set to `"true"` to enable; any other value (or unset) disables.

## API Source Configuration

To use a remote API for flag values, pass a config to the provider:

```tsx
<FeatureFlagProvider
  config={{
    apiSource: {
      url: "https://api.example.com/feature-flags",
      headers: { Authorization: "Bearer ..." },
      pollingInterval: 60000, // ms, defaults to 60s
    },
  }}
>
  {children}
</FeatureFlagProvider>
```

The API must return a JSON object with boolean values:

```json
{
  "newVaultUI": true,
  "dashboardV2": false
}
```

Unknown keys and non-boolean values are silently ignored.

## Testing

Run the unit tests:

```bash
node --test ./features/feature-flags/featureFlagService.test.cjs
```

Tests cover: defaults, enable/disable/toggle, env-var overrides, API fetch (success, error, non-ok, unknown keys), initialize lifecycle, and polling cleanup.
