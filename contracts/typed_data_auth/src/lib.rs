#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u32,
    pub verifying_contract: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transfer {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
}

#[contract]
pub struct TypedDataAuth;

fn string_to_bytes(env: &Env, s: &String) -> Bytes {
    let len = s.len() as usize;
    let mut buf = [0u8; 256];
    let slice = &mut buf[..len.min(256)];
    s.copy_into_slice(slice);
    Bytes::from_slice(env, slice)
}

#[contractimpl]
impl TypedDataAuth {
    /// Authorizes a transfer using EIP-712 style typed data hashing.
    /// Requires auth from `signer` and emits an event with the message hash.
    pub fn authorize_transfer(env: Env, domain: Domain, transfer: Transfer, signer: Address) {
        signer.require_auth();

        let domain_hash = Self::compute_domain_hash(&env, &domain);
        let struct_hash = Self::compute_struct_hash(&env, &transfer);
        let message_hash = Self::compute_message_hash(&env, domain_hash, struct_hash);

        env.events().publish(
            ("transfer_authorized",),
            (signer, transfer.from, transfer.to, transfer.amount, message_hash),
        );
    }

    pub(crate) fn compute_domain_hash(env: &Env, domain: &Domain) -> Bytes {
        let mut data = Bytes::new(env);
        data.extend_from_array(b"EIP712Domain(string name,string version,u32 chainId,Address verifyingContract)");
        data.append(&string_to_bytes(env, &domain.name));
        data.append(&string_to_bytes(env, &domain.version));
        data.extend_from_array(&domain.chain_id.to_be_bytes());
        env.crypto().sha256(&data).into()
    }

    pub(crate) fn compute_struct_hash(env: &Env, transfer: &Transfer) -> Bytes {
        let mut data = Bytes::new(env);
        data.extend_from_array(b"Transfer(address from,address to,int128 amount)");
        data.extend_from_array(&transfer.amount.to_be_bytes());
        env.crypto().sha256(&data).into()
    }

    pub(crate) fn compute_message_hash(env: &Env, domain_hash: Bytes, struct_hash: Bytes) -> Bytes {
        let mut data = Bytes::new(env);
        data.extend_from_array(&[0x19, 0x01]);
        data.append(&domain_hash);
        data.append(&struct_hash);
        env.crypto().sha256(&data).into()
    }
}

#[cfg(test)]
mod test;
