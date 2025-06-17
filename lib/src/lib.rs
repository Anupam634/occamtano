use alloy_sol_types::{sol, SolType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

sol! {
    struct PublicValuesIcr {
        uint32 icr;
        uint32 collateral_amount; // USD-scaled
    }

    struct PublicValuesLiquidation {
        uint32 liquidation_threshold;
    }

    struct PublicValuesLtv {
        uint32 real_time_ltv;
    }

    struct PublicValuesBtcHoldings {
        uint64 total_btc; // Satoshis
        bytes32 org_hash; // SHA256 of org_id
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

pub fn calculate_icr(collateral_amount: u32, debt_amount: u32, btc_price_usd: u32) -> (u32, u32) {
    let collateral_usd = collateral_amount.saturating_mul(btc_price_usd);
    let icr = if debt_amount == 0 {
        u32::MAX
    } else {
        collateral_usd.saturating_mul(100).saturating_div(debt_amount)
    };
    (icr, collateral_usd)
}

pub fn compute_org_hash(org_id: &str) -> [u8; 32] {
    Sha256::digest(org_id.as_bytes()).into()
}