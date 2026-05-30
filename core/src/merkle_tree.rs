//! Merkle Tree implementation for SoroScope state commitments.
//!
//! Issue #332 — Add `verify_proof()` to verify a generated proof against the root hash.
//!
//! # Design
//!
//! Leaves are hashed with SHA-256.  Internal nodes are produced by sorting the
//! two child hashes before concatenating them (`min || max`), which makes proofs
//! order-independent and is the standard practice used by Ethereum / OpenZeppelin.
//!
//! Odd nodes at any level are promoted by hashing with themselves (`hash(x || x)`).

use sha2::{Digest, Sha256};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// One step in a Merkle inclusion proof.
///
/// Each `ProofNode` carries the *sibling* hash at that level and the side
/// (`Left` / `Right`) that the **current** path node occupies, so the verifier
/// knows which side to place the running hash when combining.
#[derive(Debug, Clone, PartialEq)]
pub struct ProofNode {
    /// The sibling hash at this level.
    pub hash: [u8; 32],
    /// Whether the path node (the one being proven) is on the left side.
    pub is_left: bool,
}

/// A complete Merkle inclusion proof for one leaf.
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// The leaf value that is being proven (pre-hash raw bytes).
    pub leaf: Vec<u8>,
    /// Sibling hashes from the leaf level up to (but not including) the root.
    pub proof: Vec<ProofNode>,
    /// The root hash this proof is valid against.
    pub root: [u8; 32],
}

// ─────────────────────────────────────────────────────────────────────────────
// MerkleTree
// ─────────────────────────────────────────────────────────────────────────────

/// A binary Merkle Tree for storing cryptographic state commitments.
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The maximum number of levels supported (informational; not enforced).
    pub levels: usize,
    /// Raw leaf data, kept so proofs can be generated after building.
    data_leaves: Vec<Vec<u8>>,
    /// All levels of the tree: `nodes[0]` = leaf hashes, `nodes[last]` = root.
    nodes: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Creates a new, empty Merkle Tree.
    pub fn new(levels: usize) -> Self {
        MerkleTree {
            root: [0u8; 32],
            levels,
            data_leaves: Vec::new(),
            nodes: Vec::new(),
        }
    }

    // ── Build ─────────────────────────────────────────────────────────────────

    /// Build the tree from a set of raw data blocks.
    ///
    /// Each block is SHA-256 hashed to form a leaf node.  Internal nodes are
    /// produced by sorting each pair of child hashes before hashing them
    /// together.  Odd nodes are promoted by hashing with themselves.
    pub fn build(&mut self, leaves: Vec<Vec<u8>>) -> Result<(), &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build tree from empty leaves.");
        }

        // Hash every leaf.
        let leaf_hashes: Vec<[u8; 32]> = leaves.iter().map(|l| Self::hash_leaf(l)).collect();

        self.data_leaves = leaves;
        self.nodes = Self::build_levels(leaf_hashes);
        self.root = *self.nodes.last().unwrap().first().unwrap();

        Ok(())
    }

    // ── Proof generation ──────────────────────────────────────────────────────

    /// Generate an inclusion proof for the leaf at `leaf_index`.
    ///
    /// Returns `Err` if the tree has not been built yet or the index is out of
    /// range.
    pub fn generate_proof(&self, leaf_index: usize) -> Result<MerkleProof, &'static str> {
        if self.nodes.is_empty() {
            return Err("Tree has not been built yet. Call build() first.");
        }
        if leaf_index >= self.data_leaves.len() {
            return Err("Leaf index out of range.");
        }

        let mut proof_nodes: Vec<ProofNode> = Vec::new();
        let mut current_index = leaf_index;

        // Walk from the leaf level up to (but not including) the root level.
        for level in 0..self.nodes.len() - 1 {
            let level_nodes = &self.nodes[level];
            let sibling_index = if current_index % 2 == 0 {
                // Current node is on the left; sibling is to the right.
                // If there is no right sibling the node was promoted (paired
                // with itself), so the sibling hash equals the current node.
                if current_index + 1 < level_nodes.len() {
                    current_index + 1
                } else {
                    current_index // promoted — sibling is itself
                }
            } else {
                // Current node is on the right; sibling is to the left.
                current_index - 1
            };

            proof_nodes.push(ProofNode {
                hash: level_nodes[sibling_index],
                is_left: current_index % 2 == 0, // true  = path node is left
            });

            current_index /= 2;
        }

        Ok(MerkleProof {
            leaf: self.data_leaves[leaf_index].clone(),
            proof: proof_nodes,
            root: self.root,
        })
    }

    // ── Proof verification ────────────────────────────────────────────────────

    /// Verify a `MerkleProof` against a known `root` hash.
    ///
    /// # Arguments
    /// * `proof`  — The proof returned by `generate_proof()`.
    /// * `root`   — The trusted root hash to verify against.
    ///
    /// # Returns
    /// `true` if the proof is valid (the leaf is included in the tree whose
    /// root matches `root`), `false` otherwise.
    ///
    /// # Algorithm
    /// Starting from the leaf hash, each `ProofNode` tells us the sibling hash
    /// and which side the current running hash sits on.  We combine them with
    /// `combine_hashes()` and repeat until we reach the root.  The proof is
    /// valid iff the computed root equals the supplied `root`.
    pub fn verify_proof(proof: &MerkleProof, root: &[u8; 32]) -> bool {
        // Start from the hash of the raw leaf data.
        let mut running_hash = Self::hash_leaf(&proof.leaf);

        for node in &proof.proof {
            running_hash = if node.is_left {
                // Path node is left, sibling is right.
                Self::combine_hashes(&running_hash, &node.hash)
            } else {
                // Path node is right, sibling is left.
                Self::combine_hashes(&node.hash, &running_hash)
            };
        }

        &running_hash == root
    }

    // ── Getters ───────────────────────────────────────────────────────────────

    /// Returns the root hash as a lowercase hex string.
    pub fn get_root_hex(&self) -> String {
        hex::encode(self.root)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// SHA-256 hash of a raw leaf value.
    fn hash_leaf(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }

    /// Combine two child hashes into a parent hash.
    ///
    /// Hashes are sorted before concatenation so the result is the same
    /// regardless of the order in which the children are supplied by the
    /// caller.  This is the standard approach used by OpenZeppelin's
    /// MerkleProof library and prevents second-preimage attacks.
    fn combine_hashes(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut combined = Vec::with_capacity(64);
        // Sort so combine(a, b) == combine(b, a).
        if left <= right {
            combined.extend_from_slice(left);
            combined.extend_from_slice(right);
        } else {
            combined.extend_from_slice(right);
            combined.extend_from_slice(left);
        }
        let mut hasher = Sha256::new();
        hasher.update(&combined);
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }

    /// Build all tree levels from the leaf hashes up to the root.
    ///
    /// Returns a `Vec` where index 0 is the leaf level and the last element
    /// is a single-element vec containing the root hash.
    fn build_levels(leaf_hashes: Vec<[u8; 32]>) -> Vec<Vec<[u8; 32]>> {
        let mut all_levels: Vec<Vec<[u8; 32]>> = Vec::new();
        let mut current_level = leaf_hashes;

        loop {
            all_levels.push(current_level.clone());

            if current_level.len() == 1 {
                break;
            }

            let mut next_level: Vec<[u8; 32]> = Vec::new();
            let mut i = 0;

            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    // Odd node: promote by pairing with itself.
                    current_level[i]
                };
                next_level.push(Self::combine_hashes(&left, &right));
                i += 2;
            }

            current_level = next_level;
        }

        all_levels
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_tree(leaves: Vec<&str>) -> MerkleTree {
        let mut tree = MerkleTree::new(32);
        let data: Vec<Vec<u8>> = leaves.iter().map(|s| s.as_bytes().to_vec()).collect();
        tree.build(data).expect("build should succeed");
        tree
    }

    // ── Build tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_build_single_leaf() {
        let tree = make_tree(vec!["only"]);
        // Root should equal the hash of the single leaf.
        let expected = {
            let mut h = sha2::Sha256::new();
            h.update(b"only");
            let r = h.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&r);
            out
        };
        assert_eq!(tree.root, expected);
    }

    #[test]
    fn test_build_two_leaves() {
        let tree = make_tree(vec!["left", "right"]);
        // Root must be non-zero.
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_even_leaves() {
        let tree = make_tree(vec!["a", "b", "c", "d"]);
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_odd_leaves() {
        let tree = make_tree(vec!["a", "b", "c"]);
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_empty_returns_error() {
        let mut tree = MerkleTree::new(32);
        assert!(tree.build(vec![]).is_err());
    }

    #[test]
    fn test_same_leaves_same_root() {
        let t1 = make_tree(vec!["x", "y", "z"]);
        let t2 = make_tree(vec!["x", "y", "z"]);
        assert_eq!(t1.root, t2.root);
    }

    #[test]
    fn test_different_leaves_different_root() {
        let t1 = make_tree(vec!["a", "b"]);
        let t2 = make_tree(vec!["a", "c"]);
        assert_ne!(t1.root, t2.root);
    }

    #[test]
    fn test_get_root_hex_length() {
        let tree = make_tree(vec!["hello", "world"]);
        assert_eq!(tree.get_root_hex().len(), 64);
    }

    // ── generate_proof tests ──────────────────────────────────────────────────

    #[test]
    fn test_generate_proof_before_build_returns_error() {
        let tree = MerkleTree::new(32);
        assert!(tree.generate_proof(0).is_err());
    }

    #[test]
    fn test_generate_proof_out_of_range_returns_error() {
        let tree = make_tree(vec!["a", "b"]);
        assert!(tree.generate_proof(5).is_err());
    }

    #[test]
    fn test_generate_proof_single_leaf_has_no_nodes() {
        let tree = make_tree(vec!["solo"]);
        let proof = tree.generate_proof(0).unwrap();
        // No siblings — proof path is empty.
        assert!(proof.proof.is_empty());
    }

    #[test]
    fn test_generate_proof_two_leaves_has_one_node() {
        let tree = make_tree(vec!["a", "b"]);
        let proof = tree.generate_proof(0).unwrap();
        assert_eq!(proof.proof.len(), 1);
    }

    // ── verify_proof tests ────────────────────────────────────────────────────

    /// Core requirement from Issue #332: successful verification.
    #[test]
    fn test_verify_proof_valid_two_leaves() {
        let tree = make_tree(vec!["alice", "bob"]);
        let proof = tree.generate_proof(0).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_right_leaf() {
        let tree = make_tree(vec!["alice", "bob"]);
        let proof = tree.generate_proof(1).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_four_leaves_all_indices() {
        let tree = make_tree(vec!["w", "x", "y", "z"]);
        for i in 0..4 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof for leaf {} should be valid",
                i
            );
        }
    }

    #[test]
    fn test_verify_proof_valid_odd_leaf_count() {
        let tree = make_tree(vec!["a", "b", "c"]);
        for i in 0..3 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof for leaf {} should be valid",
                i
            );
        }
    }

    #[test]
    fn test_verify_proof_valid_single_leaf() {
        let tree = make_tree(vec!["lone"]);
        let proof = tree.generate_proof(0).unwrap();
        assert!(MerkleTree::verify_proof(&proof, &tree.root));
    }

    #[test]
    fn test_verify_proof_valid_large_tree() {
        let leaves: Vec<&str> = vec![
            "leaf0", "leaf1", "leaf2", "leaf3",
            "leaf4", "leaf5", "leaf6", "leaf7",
        ];
        let tree = make_tree(leaves);
        for i in 0..8 {
            let proof = tree.generate_proof(i).unwrap();
            assert!(
                MerkleTree::verify_proof(&proof, &tree.root),
                "proof for leaf {} should be valid",
                i
            );
        }
    }

    /// Tampered leaf must fail verification.
    #[test]
    fn test_verify_proof_tampered_leaf_fails() {
        let tree = make_tree(vec!["alice", "bob"]);
        let mut proof = tree.generate_proof(0).unwrap();
        proof.leaf = b"mallory".to_vec(); // swap the leaf
        assert!(!MerkleTree::verify_proof(&proof, &tree.root));
    }

    /// Tampered sibling hash must fail verification.
    #[test]
    fn test_verify_proof_tampered_sibling_fails() {
        let tree = make_tree(vec!["alice", "bob"]);
        let mut proof = tree.generate_proof(0).unwrap();
        proof.proof[0].hash = [0xFFu8; 32]; // corrupt sibling
        assert!(!MerkleTree::verify_proof(&proof, &tree.root));
    }

    /// Wrong root must fail verification.
    #[test]
    fn test_verify_proof_wrong_root_fails() {
        let tree = make_tree(vec!["alice", "bob"]);
        let proof = tree.generate_proof(0).unwrap();
        let wrong_root = [0xABu8; 32];
        assert!(!MerkleTree::verify_proof(&proof, &wrong_root));
    }

    /// Proof from one tree must not verify against a different tree's root.
    #[test]
    fn test_verify_proof_cross_tree_fails() {
        let tree1 = make_tree(vec!["a", "b"]);
        let tree2 = make_tree(vec!["c", "d"]);
        let proof = tree1.generate_proof(0).unwrap();
        assert!(!MerkleTree::verify_proof(&proof, &tree2.root));
    }
}