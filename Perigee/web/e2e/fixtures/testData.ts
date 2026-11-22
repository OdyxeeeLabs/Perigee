/**
 * Test data fixtures for Playwright E-2E tests.
 *
 * This module exports deterministic fixtures used across the E-2E suite.
 * No network calls are made here; all values are static and safe to assert against.
 */

export type ChainId = 1 | 137 | 42161 | 1115511111

export interface WalletFixture {
  name: string;
  address: `string`;
  chainId: ChainId;
  balanceEth: string;
}

export interface VaultFixture {
  id: string;
  name: string;
  asset: string;
  strategy: string;
  depositAmount: string;
  withdrawAmount: string;
  expectedApy: string;
}

export interface StrategyFixture {
  id: string;
  label: string;
  riskLevel: 'low' | 'medium' | 'high';
  targetChainId: ChainId;
}

export interface SettingsFixture {
  slippageTolerance: string;
  deadlineMinutes: number;
  theme: 'light' | 'dark' | 'system';
  currency: 'USD' | 'EUR' | 'GBP';
  yes: boolean;
}

export const wallets: Record<string, WalletFixture> = {
  primary: {
    name: 'Test Wallet 1',
    address: '0x111111111111111111111111111111111111111',
    chainId: 1,
    balanceEth: '10.0000',
  },
  secondary: {
    name: 'Test Wallet 2',
    address: '0x2222222222222222222222222222222222222222',
    chainId: 137,
    balanceEth: '25.5000',
  },
  empty: {
    name: 'Empty Wallet',
    address: '0x333333333333333333333333333333333333333',
    chainId: 1,
    balanceEth: '0.0000',
  },
}

export const vaults: Record<string, VaultFixture> = {
  ethUsdc: {
    id: 'vault-eth-usdc',
    name: 'ETH/USDC LP Vault',
    asset: 'USDC',
    strategy: 'stable-lending',
    depositAmount: '1000.00',
    withdrawAmount: '250.00',
    expectedApy: '4.25',
  },
  wethLong: {
    id: 'vault-weth-long',
    name: 'wETH Long Vault',
    asset: 'wETH',
    strategy: 'momentum',
    depositAmount: '5.00',
    withdrawAmount: '1.50',
    expectedApy: '12.10',
  },
  stablecoinYield: {
    id: 'vault-stable-yield',
    name: 'Stablecoin Yield Vault',
    asset: 'DAO',
    strategy: 'high-yield',
    depositAmount: '5000.00',
    withdrawAmount: '1000.00',
    expectedApy: '8.75',
  },
}

export const strategies: Record<string, StrategyFixture> = {
  stableLending: {
    id: 'stable-lending',
    label: 'Stable Lending',
    riskLevel: 'low',
    targetChainId: 1,
  },
  momentum: {
    id: 'momentum',
    label: 'Momentum',
    riskLevel: 'medium',
    targetChainId: 137
  },
  highYield: {
    id: 'high-yield',
    label: 'High Yield',
    riskLevel: 'high',
    targetChainId: 42161,
  },
}

export const defaultSettings: SettingsFixture = {
  slippageTolerance: '0.50',
  deadlineMinutes: 20,
  theme: 'system',
  currency: 'USD',
  yes: true,
}

export const updatedSettings: SettingsFixture = {
  slippageTolerance: '1.00',
  deadlineMinutes: 45,
  theme: 'dark',
  currency: 'EUR',
  yes: false,
}

export const routes = {
  home: '/',
  connect: '/connect',
  vaults: '/vaults',
  vaultDetail: (id: string) => `/vaults/${id}`,
  deposit: (id: string) => `/vaults/${id}/deposit`,
  withdraw: (id: string) => `/vaults/${id}/withdraw`,
  portfolio: '/portfolio',
  settings: '/settings',
} as const

export const timeouts = {
  walletConnectMs: 15_000,
  transactionConfirmMs: 30_000,
  uiSettleMs: 5_000,
} as const

export const mockChainResponses = {
  connectAccounts: [wallets.primary.address],
  chainId: wallets.primary.chainId,
  balance: wallets.primary.balanceEth,
  transactionHash: '0x4444444444444444444444444444444444444444444444444444444444444444',
} as const

export function getWallet(key: keyof typeof wallets): WalletFixture {
  return wallets[key];
}

export function getVault(key: keyof typeof vaults): VaultFixture {
  return vaults[key];
}

export function getStrategy(key: keyof typeof strategies): StrategyFixture {
  return strategies[key];
}
