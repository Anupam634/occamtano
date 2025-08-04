use sp1_sdk::Prover;
use std::str::FromStr;
use actix_web::{post, web, App, HttpResponse, HttpServer, Responder, Error as ActixError};
use alloy_sol_types::SolType;
use bitcoin::{Address, Network, PublicKey};
use log::{error, info};
use reqwest;
use serde::{Deserialize, Serialize, Deserializer};
use serde_json::Value;
use sp1_sdk::{include_elf, ProverClient, SP1Stdin, HashableKey};
use fibonacci_lib::PublicValuesBtcHoldings;
use anyhow::Result;
use hex;

// SP1 ELF binary
pub const DOGE_DEPOSIT_ELF: &[u8] = include_elf!("btc-holdings-program");

// Hardcoded receiving wallet address (Dogecoin address)
const RECEIVING_WALLET_ADDRESS: &str = "npyjkNHtqeCqRf3o1wairchdZEuyw8exj5";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DogeDepositRequest {
    tx_hash: String,
    proof_system: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DogeDepositResponse {
    sender_address: String,
    deposit_amount: u64,
    wallet_address: String,
    vkey: String,
    public_values: String,
    proof: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumTransaction {
    hash: String,
    #[serde(default)]
    locktime: u64,
    #[serde(rename = "vout", default)]
    outputs: Vec<TatumOutput>,
    #[serde(rename = "vin", default)]
    inputs: Vec<TatumInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumInput {
    #[serde(default)]
    script_sig: TatumScriptSig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TatumScriptSig {
    #[serde(default)]
    asm: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumOutput {
    #[serde(default, deserialize_with = "deserialize_script_pubkey_address")]
    address: String,
    #[serde(deserialize_with = "deserialize_value")]
    value: String,
}

fn deserialize_value<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Object(obj) => {
            if let Some(amount) = obj.get("amount") {
                match amount {
                    Value::String(s) => Ok(s.clone()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err(serde::de::Error::custom("Invalid value.amount type")),
                }
            } else {
                Err(serde::de::Error::custom("Missing amount in value object"))
            }
        }
        _ => Err(serde::de::Error::custom("Invalid value type")),
    }
}

fn deserialize_script_pubkey_address<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    value
        .get("scriptPubKey")
        .and_then(|spk| spk.get("addresses"))
        .and_then(|addrs| addrs.as_array())
        .and_then(|arr| arr.get(0))
        .and_then(|a| a.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| serde::de::Error::custom("Missing or invalid address in scriptPubKey"))
}

fn pubkey_to_doge_address(pubkey: &[u8]) -> Result<String> {
    let pubkey = PublicKey::from_slice(pubkey)?;
    let address = Address::p2pkh(&pubkey, Network::Testnet);
    Ok(address.to_string())
}

fn doge_address_to_bytes(address: &str) -> Result<Vec<u8>> {
    let addr = Address::from_str(address)?.require_network(Network::Testnet)?.script_pubkey();
    Ok(addr.as_bytes()[3..23].to_vec())
}

async fn fetch_doge_transaction(tx_hash: &str) -> Result<TatumTransaction, Box<dyn std::error::Error>> {
    let url = format!("https://api.tatum.io/v3/dogecoin/transaction/{}", tx_hash);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("x-api-key", "t-67ae0be674c77aa851dd5cce-bd0e33fb85f646a1931d9d0a")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("Failed to fetch transaction: {}", resp.status()).into());
    }

    Ok(resp.json().await?)
}

#[post("/prove-doge-deposit")]
async fn prove_doge_deposit(req: web::Json<DogeDepositRequest>) -> Result<impl Responder, ActixError> {
    info!("Received Dogecoin deposit proof request: {:?}", req);

    if !["plonk", "groth16"].contains(&req.proof_system.as_str()) {
        return Ok(HttpResponse::BadRequest().body("Invalid proof system"));
    }

    let tx = fetch_doge_transaction(&req.tx_hash).await.map_err(|e| {
        error!("Transaction fetch failed: {:?}", e);
<HttpResponse as Into<T>>::into(HttpResponse::InternalServerError().body(format!("Transaction fetch failed: {}", e)))
    })?;

    let sender_pubkey = tx.inputs.first()
        .and_then(|input| input.script_sig.asm.split_whitespace().last())
        .ok_or_else(|| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Missing public key in scriptSig")))?;;

    let sender_pubkey = hex::decode(sender_pubkey)
        .map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Invalid public key hex")))?;

    let sender_address = pubkey_to_doge_address(&sender_pubkey)
        .map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::InternalServerError().body("Failed to derive sender address")))?;

    let deposit_amount: u64 = tx.outputs
        .iter()
        .filter(|output| output.address == RECEIVING_WALLET_ADDRESS)
        .map(|output| output.value.replace(",", "").parse::<f64>().unwrap_or(0.0))
        .map(|v| (v * 100_000_000.0) as u64)
        .sum();

    if deposit_amount == 0 {
        return Ok(HttpResponse::BadRequest().body("No deposit found to the specified wallet address"));
    }

    let sender_address_bytes = doge_address_to_bytes(&sender_address)
        .map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Invalid sender address")))?;
    let wallet_address_bytes = doge_address_to_bytes(RECEIVING_WALLET_ADDRESS)
        .map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Invalid wallet address")))?;

    let client = ProverClient::builder().network().build();
    let (pk, vk) = client.setup(DOGE_DEPOSIT_ELF);

    let mut stdin = SP1Stdin::new();
    stdin.write_slice(&sender_address_bytes);
    stdin.write_slice(&deposit_amount.to_le_bytes());
    stdin.write_slice(&wallet_address_bytes);

    let proof_result = match req.proof_system.as_str() {
        "plonk" => client.prove(&pk, &stdin).plonk().run(),
        "groth16" => client.prove(&pk, &stdin).groth16().run(),
        _ => unreachable!(),
    };

    let proof = proof_result.map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Proof generation failed")))?;

    let public_bytes = proof.public_values.as_slice();
    let public_values = PublicValuesBtcHoldings::abi_decode(public_bytes)
        .map_err(|_| <HttpResponse as Into<T>>::into(HttpResponse::BadRequest().body("Failed to decode public values")))?;

    Ok(HttpResponse::Ok().json(DogeDepositResponse {
        sender_address,
        deposit_amount: public_values.deposit_amount,
        wallet_address: RECEIVING_WALLET_ADDRESS.to_string(),
        vkey: vk.bytes32(),
        public_values: format!("0x{}", hex::encode(public_bytes)),
        proof: format!("0x{}", hex::encode(proof.bytes())),
    }))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    info!("Starting Dogecoin Deposit SP1 proof server on http://localhost:3005");

    HttpServer::new(|| App::new().service(prove_doge_deposit))
        .workers(4)
        .bind(("0.0.0.0", 3005))?
        .run()
        .await
}
