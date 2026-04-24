#![cfg(test)]
//! Property-based fuzz tests for the CPU-heavy contract.
//!
//! Verifies determinism, correctness invariants, and absence of panics
//! across a wide range of inputs for computationally intensive functions.

use super::*;
use proptest::prelude::*;
use soroban_sdk::{Env, Vec};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Fibonacci must be deterministic: same input → same output.
    /// Must not panic for any value in the valid range [0, MAX_FIB].
    #[test]
    fn fuzz_fibonacci_deterministic(n in 0u32..=MAX_FIB) {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let contract_id = env.register(CpuHeavyContract, ());
        let client = CpuHeavyContractClient::new(&env, &contract_id);

        let result1 = client.fibonacci_iterative(&n);
        let result2 = client.fibonacci_iterative(&n);
        prop_assert_eq!(result1, result2, "Fibonacci is not deterministic for n={}", n);
    }

    /// Bubble sort must produce a sorted output of the same length.
    /// The output must contain the same elements as the input (permutation).
    #[test]
    fn fuzz_bubble_sort_is_sorted(values in prop::collection::vec(any::<u32>(), 0..=MAX_SORT as usize)) {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let contract_id = env.register(CpuHeavyContract, ());
        let client = CpuHeavyContractClient::new(&env, &contract_id);

        let mut soroban_vec = Vec::new(&env);
        for v in &values {
            soroban_vec.push_back(*v);
        }

        let sorted = client.bubble_sort(&soroban_vec);

        // Length invariant
        prop_assert_eq!(
            sorted.len(),
            soroban_vec.len(),
            "Sorted output has different length"
        );

        // Sorted invariant: each element <= the next
        for i in 1..sorted.len() {
            let prev = sorted.get(i - 1).unwrap();
            let curr = sorted.get(i).unwrap();
            prop_assert!(
                prev <= curr,
                "Output not sorted at index {}: {} > {}",
                i, prev, curr
            );
        }

        // Permutation invariant: same elements when both are sorted
        let mut input_sorted: std::vec::Vec<u32> = values.clone();
        input_sorted.sort();
        let mut output_collected: std::vec::Vec<u32> = (0..sorted.len())
            .map(|i| sorted.get(i).unwrap())
            .collect();
        output_collected.sort();
        prop_assert_eq!(
            input_sorted,
            output_collected,
            "Sorted output is not a permutation of input"
        );
    }

    /// `count_primes(n)` must be monotonically non-decreasing:
    /// if n1 <= n2, then count_primes(n1) <= count_primes(n2).
    #[test]
    fn fuzz_count_primes_monotonic(n in 2u32..=MAX_PRIME - 1) {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let contract_id = env.register(CpuHeavyContract, ());
        let client = CpuHeavyContractClient::new(&env, &contract_id);

        let count_n = client.count_primes(&n);
        let count_n1 = client.count_primes(&(n + 1));

        prop_assert!(
            count_n <= count_n1,
            "count_primes is not monotonic: count_primes({}) = {} > count_primes({}) = {}",
            n, count_n, n + 1, count_n1
        );
    }
}
