use serde::Serialize;
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
use serde::{Deserialize, Serialize};

/// Which side of the current node a proof sibling belongs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofDirection {
    Left,
    Right,
}

/// A single sibling hash in a Merkle proof path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleProofStep {
    pub direction: ProofDirection,
    pub hash: [u8; 32],
}

/// A Merkle inclusion proof for one leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf_index: usize,
    pub leaf_hash: [u8; 32],
    pub root: [u8; 32],
    pub steps: Vec<MerkleProofStep>,
}

impl MerkleProof {
    /// Verifies this proof against the stored root.
    pub fn verify(&self) -> bool {
        let mut current = self.leaf_hash;

        for step in &self.steps {
            current = match step.direction {
                ProofDirection::Left => MerkleTree::hash_pair(&step.hash, &current),
                ProofDirection::Right => MerkleTree::hash_pair(&current, &step.hash),
            };
        }

        current == self.root
    }
}

#[derive(Debug, Serialize)]
pub struct MerkleProofItem {
    pub sibling_hash: String,
    pub sibling_is_left: bool,
}
use rayon::prelude::*;

/// Represents a Merkle Tree for storing cryptographic state commitments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The maximum supported tree depth.
    /// The maximum number of tree levels.
    pub levels: usize,
    /// The raw leaf inputs.
    data_leaves: Vec<Vec<u8>>,
    /// Cached hashed levels. Index 0 contains leaf hashes, the last index contains the root.
    hashed_levels: Vec<Vec<[u8; 32]>>,
    /// All tree levels from leaves to the root.
    tree_levels: Vec<Vec<[u8; 32]>>,
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
            hashed_levels: Vec::new(),
        }
    }

    /// Builds the Merkle Tree from a provided set of data blocks.
            tree_levels: Vec::new(),
        }
    }

    /// Builds the Merkle Tree from a provided set of leaf values.
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
    /// The root hash of the tree (hex-encoded string).
    pub root: String,
    /// The number of leaves in the tree.
    pub leaf_count: usize,
    /// The depth of the tree.
    pub depth: usize,
}

impl MerkleTree {
    /// Creates a new Merkle Tree from a set of data leaves.
    /// 
    /// # Arguments
    /// * `leaves` - Vector of byte vectors representing the data to hash
    /// 
    /// # Returns
    /// A new MerkleTree with the computed root hash, or an error if leaves are empty
    pub fn new(leaves: Vec<Vec<u8>>) -> Result<Self, &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build tree from empty leaves.");
        }

        let leaf_capacity = 1usize
            .checked_shl(self.levels as u32)
            .ok_or("Tree levels exceed supported usize capacity.")?;

        if leaves.len() > leaf_capacity {
            return Err("Leaf count exceeds configured tree capacity.");
        }

        let hashed_levels = Self::calculate_levels(&leaves);
        let root = hashed_levels
            .last()
            .and_then(|level| level.first())
            .copied()
            .ok_or("Cannot calculate a root for empty levels.")?;

        self.root = root;
        self.data_leaves = leaves;
        self.hashed_levels = hashed_levels;
        // Hash every leaf.
        let leaf_hashes: Vec<[u8; 32]> = leaves.iter().map(|l| Self::hash_leaf(l)).collect();

        self.data_leaves = leaves;
        self.nodes = Self::build_levels(leaf_hashes);
        self.root = *self.nodes.last().unwrap().first().unwrap();

        Ok(())
    }

    /// Generates an inclusion proof for the leaf at `leaf_index`.
    pub fn generate_proof(&self, leaf_index: usize) -> Result<MerkleProof, &'static str> {
        if self.hashed_levels.is_empty() {
            return Err("Cannot generate proof before building the tree.");
        }

        if leaf_index >= self.data_leaves.len() {
            return Err("Leaf index is out of bounds.");
        }

        let mut proof_index = leaf_index;
        let mut steps = Vec::new();

        for level in self
            .hashed_levels
            .iter()
            .take(self.hashed_levels.len().saturating_sub(1))
        {
            let is_right_node = proof_index % 2 == 1;
            let sibling_index = if is_right_node {
                proof_index - 1
            } else {
                proof_index + 1
            };

            let sibling_hash = level
                .get(sibling_index)
                .copied()
                .unwrap_or_else(|| level[proof_index]);

            steps.push(MerkleProofStep {
                direction: if is_right_node {
                    ProofDirection::Left
                } else {
                    ProofDirection::Right
                },
                hash: sibling_hash,
            });

            proof_index /= 2;
        }

        Ok(MerkleProof {
            leaf_index,
            leaf_hash: self.hashed_levels[0][leaf_index],
            root: self.root,
            steps,
        })
        let leaf_count = leaves.len();
        let depth = Self::calculate_depth(leaf_count);
        
        // Hash each leaf with SHA256
        let hashed_leaves: Vec<Vec<u8>> = leaves
            .into_iter()
            .map(|leaf| Self::hash_leaf(&leaf))
            .collect();

        // Calculate the root from the hashed leaves
        let root_hash = Self::calculate_root_hash(hashed_leaves)?;
        let root = hex::encode(root_hash);

        Ok(MerkleTree {
            root,
            leaf_count,
            depth,
        })
    }

    /// Creates a Merkle Tree from hex-encoded leaf data strings.
    /// Useful when working with state snapshots.
    pub fn from_hex_strings(hex_leaves: Vec<String>) -> Result<Self, &'static str> {
        let leaves: Result<Vec<Vec<u8>>, &'static str> = hex_leaves
            .into_iter()
            .map(|hex| hex::decode(&hex).map_err(|_| "Invalid hex encoding in leaf data"))
            .collect();
        
        Self::new(leaves?)
    }

    /// Calculates the tree depth (number of levels) given a leaf count.
    fn calculate_depth(leaf_count: usize) -> usize {
        if leaf_count == 0 {
            return 0;
        }
        (leaf_count as f64).log2().ceil() as usize + 1
    }

    /// Hashes a single leaf using SHA256.
    fn hash_leaf(data: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().to_vec()
    }

    /// Hashes two sibling nodes by concatenating and hashing with SHA256.
    fn hash_pair(left: &[u8], right: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().to_vec()
    }

    /// Recursively calculates the root hash of the Merkle tree.
    fn calculate_root_hash(mut current_level: Vec<Vec<u8>>) -> Result<Vec<u8>, &'static str> {
        if current_level.is_empty() {
            return Err("Cannot calculate root of empty tree.");
        }

        // Continue hashing pairs until only one hash remains (the root)
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            let pairs = current_level.len() / 2;

            // Process pairs of hashes
            for i in 0..pairs {
                let left = &current_level[i * 2];
                let right = &current_level[i * 2 + 1];
                next_level.push(Self::hash_pair(left, right));
            }

            // If there's an odd number of nodes, duplicate the last one
            if current_level.len() % 2 != 0 {
                let last = &current_level[current_level.len() - 1];
                next_level.push(Self::hash_pair(last, last));
        // Hash each leaf first
        let hashed: Vec<Vec<u8>> = leaves
            .iter()
            .map(|leaf| {
                let mut h = Sha256::new();
                h.update(leaf);
                h.finalize().to_vec()
            })
            .collect();

        self.data_leaves = leaves;
        self.root = Self::calculate_root_hash(hashed);
        Ok(())
    }

    /// Iteratively hashes adjacent node pairs up to the root level.
    /// Odd nodes are hashed with themselves (hash(x || x)).
    fn calculate_root_hash(mut current_level: Vec<Vec<u8>>) -> [u8; 32] {
        while current_level.len() > 1 {
            let mut next_level: Vec<Vec<u8>> = Vec::new();

            let mut i = 0;
            while i < current_level.len() {
                let left = &current_level[i];
                // If no right sibling, duplicate the left node
                let right = if i + 1 < current_level.len() {
                    &current_level[i + 1]
                } else {
                    left
                };

                let mut hasher = Sha256::new();
                hasher.update(left);
                hasher.update(right);
                next_level.push(hasher.finalize().to_vec());

                i += 2;
        let max_leaves = if self.levels >= 64 {
            usize::MAX
        } else {
            1usize.checked_shl(self.levels as u32).unwrap_or(usize::MAX)
        };

        if leaves.len() > max_leaves {
            return Err("Number of leaves exceeds maximum for configured tree levels.");
        }

        self.data_leaves = leaves;
        let mut current_level: Vec<[u8; 32]> = self
            .data_leaves
            .iter()
            .map(|leaf| Self::hash_leaf(leaf))
            .collect();

        self.tree_levels.clear();
        self.tree_levels.push(current_level.clone());

        while current_level.len() > 1 {
            current_level = Self::parent_level(&current_level);
            self.tree_levels.push(current_level.clone());
        
        self.data_leaves = leaves.clone();
        self.root.copy_from_slice(&Self::calculate_root_hash(leaves));
        Ok(())
    }

    /// A helper function to perform the recursive root calculation.
    fn calculate_root_hash(mut levels: Vec<Vec<u8>>) -> [u8; 32] {
        while levels.len() > 1 {
            let next_level: Vec<Vec<u8>> = levels
                .par_chunks(2)
                .map(|pair| {
                    let mut hasher = Sha256::new();
                    hasher.update(&pair[0]);
                    if pair.len() == 2 {
                        hasher.update(&pair[1]);
                    } else {
                        hasher.update(&pair[0]); // Hash with itself if odd
                    }
                    hasher.finalize().to_vec()
                })
                .collect();
            levels = next_level;
        }
        
        // Return the final hash (the root)
        let mut root_bytes = [0u8; 32];
        if let Some(root) = levels.first() {
            root_bytes.copy_from_slice(root);
        }

        self.root = current_level[0];
        Ok(())
    }

    fn hash_leaf(leaf: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(leaf);
        hasher.finalize().into()
    }

    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }

    fn parent_level(current_level: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let mut parent = Vec::new();
        let mut i = 0;

        while i < current_level.len() {
            let left = &current_level[i];
            let right = if i + 1 < current_level.len() {
                &current_level[i + 1]
            } else {
                &current_level[i]
            };

            parent.push(Self::hash_pair(left, right));
            i += 2;
        }

        parent
    }

    /// Generates a proof for the leaf at the specified index.
    pub fn generate_proof(&self, leaf_index: usize) -> Result<Vec<MerkleProofItem>, &'static str> {
        if self.tree_levels.is_empty() {
            return Err("Merkle tree has not been built.");
        }
        if leaf_index >= self.tree_levels[0].len() {
            return Err("Leaf index out of bounds.");
        }

        let mut proof = Vec::new();
        let mut index = leaf_index;

        for level in &self.tree_levels[..self.tree_levels.len() - 1] {
            let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };
            let (sibling_hash, sibling_is_left) = if sibling_index < level.len() {
                (level[sibling_index], sibling_index < index)
            } else {
                (level[index], false)
            };

            proof.push(MerkleProofItem {
                sibling_hash: hex::encode(sibling_hash),
                sibling_is_left,
            });
            index /= 2;
        }

        Ok(proof)
    }

    /// Verifies that a leaf and proof correspond to the expected root hash.
    pub fn verify_proof(
        leaf: &[u8],
        proof: &[MerkleProofItem],
        expected_root_hex: &str,
    ) -> Result<bool, &'static str> {
        let mut computed_hash = Self::hash_leaf(leaf);

        for item in proof {
            let sibling_bytes =
                hex::decode(&item.sibling_hash).map_err(|_| "Invalid proof sibling hash hex")?;
            if sibling_bytes.len() != 32 {
                return Err("Invalid proof sibling hash length");
            }
            let mut sibling_hash = [0u8; 32];
            sibling_hash.copy_from_slice(&sibling_bytes);

            current_level = next_level;
        }

        Ok(current_level.into_iter().next().unwrap())
        let mut root = [0u8; 32];
        if let Some(r) = current_level.first() {
            root.copy_from_slice(r);
        }
        root
    }

    /// Gets the root hash as a hex string.
            computed_hash = if item.sibling_is_left {
                Self::hash_pair(&sibling_hash, &computed_hash)
            } else {
                Self::hash_pair(&computed_hash, &sibling_hash)
            };
        }

        let expected_root_bytes =
            hex::decode(expected_root_hex).map_err(|_| "Invalid expected root hex")?;
        if expected_root_bytes.len() != 32 {
            return Err("Invalid expected root length");
        }

        let mut expected_root = [0u8; 32];
        expected_root.copy_from_slice(&expected_root_bytes);

        Ok(computed_hash == expected_root)
    }

    /// Returns the Merkle tree root as a hex string.
    pub fn get_root_hex(&self) -> String {
        hex::encode(self.root)
    }

    fn calculate_levels(leaves: &[Vec<u8>]) -> Vec<Vec<[u8; 32]>> {
        let mut levels = vec![leaves
            .iter()
            .map(|leaf| Self::hash_leaf(leaf))
            .collect::<Vec<_>>()];

        while levels.last().map_or(0, Vec::len) > 1 {
            let current_level = levels.last().expect("level exists");
            let mut next_level = Vec::new();

            for pair in current_level.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(left);
                next_level.push(Self::hash_pair(&left, &right));
            }

            levels.push(next_level);
        }

        levels
    }

    fn hash_leaf(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
        self.root.clone()
    /// Returns the number of leaves contained in the tree.
    pub fn leaf_count(&self) -> usize {
        self.data_leaves.len()
    }
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

    #[test]
    fn builds_tree_and_generates_valid_proofs_for_each_leaf() {
        let mut tree = MerkleTree::new(3);
        let leaves = vec![
            b"cpu-budget".to_vec(),
            b"read-bytes".to_vec(),
            b"write-bytes".to_vec(),
            b"events".to_vec(),
            b"ledger-footprint".to_vec(),
        ];

        tree.build(leaves).expect("tree builds");

        for index in 0..5 {
            let proof = tree.generate_proof(index).expect("proof exists");
            assert_eq!(proof.leaf_index, index);
            assert_eq!(proof.root, tree.root);
            assert!(proof.verify());
        }
    fn test_merkle_tree_single_leaf() {
        let data = vec![b"data1".to_vec()];
        let tree = MerkleTree::new(data).expect("Failed to build tree");
        
        assert_eq!(tree.leaf_count, 1);
        assert!(!tree.root.is_empty());
        assert!(tree.root.len() > 0); // Hex encoded hash
    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_tree(leaves: Vec<&str>) -> MerkleTree {
        let mut tree = MerkleTree::new(32);
        let data: Vec<Vec<u8>> = leaves.iter().map(|s| s.as_bytes().to_vec()).collect();
        tree.build(data).expect("build should succeed");
        tree
        
        // Example data leaves
        let data = vec![b"data1".to_vec(), b"data2".to_vec(), b"data3".to_vec()];
        
        let result = tree.build(data);
        assert!(result.is_ok());
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

    #[test]
    fn test_merkle_tree_multiple_leaves() {
        let data = vec![
            b"data1".to_vec(),
            b"data2".to_vec(),
            b"data3".to_vec(),
            b"data4".to_vec(),
        ];
        let tree = MerkleTree::new(data).expect("Failed to build tree");
        
        assert_eq!(tree.leaf_count, 4);
        assert!(!tree.root.is_empty());
    }

    #[test]
    fn test_merkle_tree_odd_leaves() {
        let data = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let tree = MerkleTree::new(data).expect("Failed to build tree");
        
        assert_eq!(tree.leaf_count, 3);
        assert!(!tree.root.is_empty());
    }

    #[test]
    fn test_merkle_tree_empty_fails() {
        let data: Vec<Vec<u8>> = vec![];
        let result = MerkleTree::new(data);
        
        assert!(result.is_err());
    }

    #[test]
    fn test_merkle_tree_deterministic() {
        let data = vec![b"test".to_vec(), b"data".to_vec()];
        let tree1 = MerkleTree::new(data.clone()).expect("Failed to build tree 1");
        let tree2 = MerkleTree::new(data).expect("Failed to build tree 2");
        
        // Same input should produce same root
        assert_eq!(tree1.root, tree2.root);
    }

    #[test]
    fn test_merkle_tree_from_hex_strings() {
        let hex_leaves = vec![
            hex::encode(b"data1"),
            hex::encode(b"data2"),
        ];
        let tree = MerkleTree::from_hex_strings(hex_leaves).expect("Failed to build tree");
        
        assert_eq!(tree.leaf_count, 2);
        assert!(!tree.root.is_empty());
    }

    #[test]
    fn proof_verification_rejects_tampered_leaf_hash() {
        let mut tree = MerkleTree::new(2);
        tree.build(vec![b"left".to_vec(), b"right".to_vec()])
            .expect("tree builds");

        let mut proof = tree.generate_proof(0).expect("proof exists");
        proof.leaf_hash[0] ^= 0xff;

        assert!(!proof.verify());
    }

    #[test]
    fn single_leaf_proof_has_no_sibling_steps() {
        let mut tree = MerkleTree::new(1);
        tree.build(vec![b"only-leaf".to_vec()])
            .expect("tree builds");

        let proof = tree.generate_proof(0).expect("proof exists");

        assert!(proof.steps.is_empty());
        assert!(proof.verify());
    }

    #[test]
    fn rejects_out_of_bounds_proof_requests() {
        let mut tree = MerkleTree::new(2);
        tree.build(vec![b"left".to_vec(), b"right".to_vec()])
            .expect("tree builds");

        assert_eq!(
            tree.generate_proof(2).expect_err("index should fail"),
            "Leaf index is out of bounds."
        );
    fn test_build_rejects_empty_leaves() {
        let mut tree = MerkleTree::new(32);
        assert!(tree.build(vec![]).is_err());
    }

    #[test]
    fn test_build_single_leaf() {
        let mut tree = MerkleTree::new(32);
        assert!(tree.build(vec![b"data1".to_vec()]).is_ok());
        // Root should be SHA256("data1")
        let expected = {
            let mut h = Sha256::new();
            h.update(b"data1");
            hex::encode(h.finalize())
        };
        assert_eq!(tree.get_root_hex(), expected);
    }

    #[test]
    fn test_build_even_leaves() {
        let mut tree = MerkleTree::new(32);
        let data = vec![b"data1".to_vec(), b"data2".to_vec()];
        assert!(tree.build(data).is_ok());
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_build_odd_leaves() {
        let mut tree = MerkleTree::new(32);
        let data = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        assert!(tree.build(data).is_ok());
        assert_ne!(tree.root, [0u8; 32]);
    }

    #[test]
    fn test_deterministic_root() {
        let data = vec![b"x".to_vec(), b"y".to_vec()];
        let mut t1 = MerkleTree::new(32);
        let mut t2 = MerkleTree::new(32);
        t1.build(data.clone()).unwrap();
        t2.build(data).unwrap();
        assert_eq!(t1.root, t2.root);
    }
    fn test_merkle_tree_root_and_proof() {
        let leaves = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()];

        let mut tree = MerkleTree::new(32);
        tree.build(leaves.clone()).expect("Failed to build tree");

        assert_eq!(tree.leaf_count(), 4);
        assert!(!tree.get_root_hex().is_empty());

        let proof = tree.generate_proof(2).expect("Failed to generate proof");
        assert_eq!(proof.len(), 2);
        assert!(MerkleTree::verify_proof(&leaves[2], &proof, &tree.get_root_hex()).unwrap());
    }
}
