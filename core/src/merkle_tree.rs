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

/// Represents a Merkle Tree for storing cryptographic state commitments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The maximum supported tree depth.
    pub levels: usize,
    /// The current leaf nodes/data inputs.
    data_leaves: Vec<Vec<u8>>,
    /// Cached hashed levels. Index 0 contains leaf hashes, the last index contains the root.
    hashed_levels: Vec<Vec<[u8; 32]>>,
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
            }

            current_level = next_level;
        }

        Ok(current_level.into_iter().next().unwrap())
    }

    /// Gets the root hash as a hex string.
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
    }
}

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
    }
}
