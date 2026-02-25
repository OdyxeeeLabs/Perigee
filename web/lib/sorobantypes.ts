export type SorobanType = 'address' | 'u32' | 'i128' | 'u128' | 'string' | 'symbol' | 'bool' | 'struct' | 'enum';

export interface ContractFunction {
  name: string;
  inputs: ContractInput[];
  outputs?: SorobanType;
}

export interface ContractInput {
  name: string;
  type: SorobanType;
  description?: string;
  optional?: boolean;
}

export interface InvocationResult {
  id: string;
  functionName: string;
  inputs: Record<string, any>;
  result?: any;
  error?: string;
  resourceCost?: {
    fee?: string;
    cpu_instructions: number;
    ram_bytes: number;
    ledger_read_bytes: number;
    ledger_write_bytes: number;
    transaction_size_bytes: number;
  };
  timestamp: number;
  success: boolean;
}

// Mock contract functions for demo
export const MOCK_CONTRACT_FUNCTIONS: ContractFunction[] = [
  {
    name: 'transfer',
    inputs: [
      { name: 'from', type: 'address', description: 'Sender address' },
      { name: 'to', type: 'address', description: 'Recipient address' },
      { name: 'amount', type: 'u128', description: 'Amount to transfer' },
    ],
    outputs: 'bool',
  },
  {
    name: 'balance',
    inputs: [{ name: 'account', type: 'address', description: 'Account address' }],
    outputs: 'u128',
  },
  {
    name: 'mint',
    inputs: [
      { name: 'to', type: 'address', description: 'Recipient address' },
      { name: 'amount', type: 'u128', description: 'Amount to mint' },
    ],
    outputs: 'bool',
  },
  {
    name: 'symbol',
    inputs: [],
    outputs: 'string',
  },
  {
    name: 'decimals',
    inputs: [],
    outputs: 'u32',
  },
];

export function generateMockResult(functionName: string, inputs: Record<string, any>) {
  const results: Record<string, any> = {
    transfer: { success: true, transaction_hash: '0x' + Math.random().toString(16).slice(2) },
    balance: Math.floor(Math.random() * 1000000),
    mint: { success: true, amount_minted: inputs.amount },
    symbol: 'USDC',
    decimals: 6,
  };
  return results[functionName] || { success: true, message: 'Function executed' };
}

export function generateMockResourceCost() {
  return {
    fee: (Math.random() * 0.05).toFixed(5),
    cpu_instructions: Math.floor(Math.random() * 50_000_000) + 1_000_000,
    ram_bytes: Math.floor(Math.random() * 20 * 1024 * 1024) + 1024 * 1024,
    ledger_read_bytes: Math.floor(Math.random() * 10 * 1024),
    ledger_write_bytes: Math.floor(Math.random() * 5 * 1024),
    transaction_size_bytes: Math.floor(Math.random() * 2 * 1024),
  };
}
