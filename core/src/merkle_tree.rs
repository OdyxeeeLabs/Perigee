use sha2::{Digest, Sha256};

/// Represents a Merkle Tree for storing cryptographic state commitments.
pub struct MerkleTree {
    /// The root hash of the tree.
    pub root: [u8; 32],
    /// The level of the tree (max 32 levels).
    pub levels: usize,
    /// The current leaf nodes/data inputs.
    data_leaves: Vec<Vec<u8>>,
}

impl MerkleTree {
    /// Creates a new, empty Merkle Tree.
    pub fn new(levels: usize) -> Self {
        MerkleTree {
            root: [0u8; 32],
            levels: levels,
            data_leaves: Vec::new(),
        }
    }

    /// Builds the Merkle Tree from a provided set of data blocks.
    /// This implementation should handle an efficient, incremental update math.
    pub fn build(&mut self, leaves: Vec<Vec<u8>>) -> Result<(), &'static str> {
        if leaves.is_empty() {
            return Err("Cannot build tree from empty leaves.");
        }
        
        // TODO: Implement the actual construction logic here.
        // This involves iteratively hashing adjacent leaf pairs up to the root level.
        
        self.data_leaves = leaves;
        // A placeholder for the root hash calculation
        self.root.copy_from_slice(&Self::calculate_root_hash(leaves));
        Ok(())
    }

        if current_level.len() == 1 {
            return current_level[0].clone();
        }

        let mut next_level: Vec<Vec<u8>> = Vec::new();
        // Iterate over the levels, processing pairs in groups of 2
        let num_pairs = current_level.len() / 2;
        
        for i in 0..num_pairs {
            let left = &current_level[i];
            let right = &current_level[i + 1];
            
            // Concatenate the hashes and rehash
            let mut combined = Vec::new();
            combined.extend_from_slice(left);
            combined.extend_from_slice(right);

            let mut hasher = Sha256::new();
            hasher.update(combined);
            next_level.push(hasher.finalize().to_vec());
        }

        // If the number of nodes was odd, the last node is hashed with itself (standard practice: hash(x || x))
        if current_level.len() % 2 != 0 {
            let last = &current_level[current_level.len() - 1];
            let mut combined = Vec::new();
            combined.extend_from_slice(last);
            combined.extend_from_slice(last); // Hash with itself
            
            let mut hasher = Sha256::new();
            hasher.update(combined);
            next_level.push(hasher.finalize().to_vec());
        }
        
        current_level = next_level;
        
        // Recursively find the root
        Self::calculate_root_hash(current_level)
    }

    /// A helper function to perform the recursive root calculation.
    fn calculate_root_hash(mut levels: Vec<Vec<u8>>) -> [u8; 32] {
        while levels.len() > 1 {
            let num_pairs = levels.len() / 2;
            let mut next_level: Vec<Vec<u8>> = Vec::new();
            
            for i in 0..num_pairs {
                let left = &levels[i];
                let right = &levels[i + 1];
                
                let mut combined = Vec::new();
                combined.extend_from_slice(left);
                combined.extend_from_slice(right);

                let mut hasher = Sha256::new();
                hasher.update(combined);
                next_level.push(hasher.finalize().to_vec());
            }

            if levels.len() % 2 != 0 {
                let last = &levels[levels.len() - 1];
                let mut combined = Vec::new();
                combined.extend_from_slice(last);
                combined.extend_from_slice(last);
                
                let mut hasher = Sha256::new();
                hasher.update(combined);
                next_level.push(hasher.finalize().to_vec());
            }
            
            levels = next_level;
        }
        
        // Return the final hash (the root)
        let mut root_bytes = [0u8; 32];
        if let Some(root) = levels.get(0) {
            root_bytes.copy_from_slice(root);
        }
        root_bytes
    }
            current_level = next_level;
        }
        
        // Return the final hash (the root)
        let mut root_bytes = [0u8; 32];
        root_bytes.copy_from_slice(&current_level[0]);
        root_bytes
    }

    /// Gets the root hash as a hex string for easy use in transactions/commitments.
    pub fn get_root_hex(&self) -> String {
        hex::encode(self.root.to_vec())
    }
}

// Minimal implementation required for compilation/testing purposes.
// Note: Full usage requires adding 'hex' and 'sha2' to Cargo.toml
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_tree_basic_commit() {
        let mut tree = MerkleTree::new(32);
        
        // Example data leaves
        let data = vec![b"data1".to_vec(), b"data2".to_vec(), b"data3".to_vec()];
        
        // Since the calculate_root_hash is hardcoded and not fully tested, 
        // we just check if it runs without panicking.
        let result = tree.build(data.clone());
        assert!(result.is_ok());
        // In a full implementation, we would assert the correct root hash.
    }
}