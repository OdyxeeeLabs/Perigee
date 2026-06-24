use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub struct MerkleProofItem {
    pub sibling_hash: String,
    pub sibling_is_left: bool,
}

/// Represents a Merkle Tree for storing cryptographic state commitments.
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The maximum number of tree levels.
    pub levels: usize,
    /// The raw leaf inputs.
    data_leaves: Vec<Vec<u8>>,
    /// All tree levels from leaves to the root.
    tree_levels: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Creates a new, empty Merkle Tree.
    pub fn new(levels: usize) -> Self {
        MerkleTree {
            root: [0u8; 32],
            levels,
            data_leaves: Vec::new(),
            tree_levels: Vec::new(),
        }
    }

    /// Builds the Merkle Tree from a provided set of leaf values.
    pub fn build(&mut self, leaves: Vec<Vec<u8>>) -> Result<(), &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build tree from empty leaves.");
        }

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

    /// Returns the number of leaves contained in the tree.
    pub fn leaf_count(&self) -> usize {
        self.data_leaves.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
