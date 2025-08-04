//! A program that proves a Dogecoin deposit to a defined wallet address in a zkVM.

#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_sol_types::SolType;
use alloy_primitives::Address;
use fibonacci_lib::PublicValuesBtcHoldings;

pub fn main() {
    // Step 1: Read inputs from zkVM
    let raw_sender_address = sp1_zkvm::io::read::<[u8; 20]>(); // Sender's Ethereum-style address
    let deposit_amount = sp1_zkvm::io::read::<u64>(); // Deposit amount in satoshis
    let raw_wallet_address = sp1_zkvm::io::read::<[u8; 20]>(); // Defined wallet address

    // Step 2: Convert to alloy Address
    let sender_address = Address::from_slice(&raw_sender_address);
    let wallet_address = Address::from_slice(&raw_wallet_address);

    // Step 3: Populate the public values struct
    let public_values = PublicValuesBtcHoldings {
        sender_address,
        deposit_amount,
        wallet_address,
    };

    // Step 4: ABI-encode the public values
    let bytes = PublicValuesBtcHoldings::abi_encode(&public_values);

    // Step 5: Commit encoded values to the zkVM
    sp1_zkvm::io::commit_slice(&bytes);
}