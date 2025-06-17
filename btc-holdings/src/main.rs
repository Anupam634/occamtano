#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_sol_types::SolType;
use alloy_sol_types::private::FixedBytes;
use fibonacci_lib::{PublicValuesBtcHoldings, BtcHoldingsInput};
use sha2::{Digest, Sha256};

pub fn main() {
    let input: BtcHoldingsInput = sp1_zkvm::io::read();

    let mut total = 0u64;

    for utxo in input.utxos.iter() {
        // Skip pubkey and signature checks — only count the value
        total = total.checked_add(utxo.amount).expect("no overflow");
    }

    assert_eq!(total, input.expected_total, "Total BTC mismatch");

    // Hash the org_id and convert to [u8; 32]
    let org_hash_digest = Sha256::digest(input.org_id.as_bytes());
    let org_hash_array: [u8; 32] = org_hash_digest.into();
    let fixed_hash = FixedBytes::<32>::from(org_hash_array);

    let public_values = PublicValuesBtcHoldings {
        total_btc: total,
        org_hash: fixed_hash,
    };

    let bytes = PublicValuesBtcHoldings::abi_encode(&public_values);
    sp1_zkvm::io::commit_slice(&bytes);
}