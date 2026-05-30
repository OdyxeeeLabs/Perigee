use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};

/// Represents a Merkle Tree for storing cryptographic state commitments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
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
        self.root.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
}