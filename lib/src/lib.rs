use alloy_sol_types::{sol, SolType};
use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

sol! {
    struct PublicValuesBtcHoldings {
        address sender_address;
        uint64 deposit_amount;
        address wallet_address;
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Utxo {
    pub txid: [u8; 32],
    pub index: u32,
    pub amount: u64, // Satoshis
    #[serde(serialize_with = "serialize_vec_33", deserialize_with = "deserialize_vec_33")]
    pub pubkey: Vec<u8>, // Compressed pubkey (33 bytes)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BtcSignature {
    #[serde(serialize_with = "serialize_vec_64", deserialize_with = "deserialize_vec_64")]
    pub sig: Vec<u8>, // ECDSA signature (64 bytes)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BtcHoldingsInput {
    pub utxos: Vec<Utxo>,
    pub signatures: Vec<BtcSignature>,
    pub expected_total: u64, // Satoshis
    pub org_id: String,
}

fn serialize_vec_33<S>(vec: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if vec.len() != 33 {
        return Err(serde::ser::Error::custom("pubkey must be 33 bytes"));
    }
    vec.serialize(serializer)
}

fn deserialize_vec_33<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let vec = Vec::<u8>::deserialize(deserializer)?;
    if vec.len() != 33 {
        return Err(serde::de::Error::custom("pubkey must be 33 bytes"));
    }
    Ok(vec)
}

fn serialize_vec_64<S>(vec: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if vec.len() != 64 {
        return Err(serde::ser::Error::custom("signature must be 64 bytes"));
    }
    vec.serialize(serializer)
}

fn deserialize_vec_64<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let vec = Vec::<u8>::deserialize(deserializer)?;
    if vec.len() != 64 {
        return Err(serde::de::Error::custom("signature must be 64 bytes"));
    }
    Ok(vec)
}