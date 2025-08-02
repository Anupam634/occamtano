use actix_web::{post, web, App, HttpResponse, HttpServer, Responder, Error as ActixError};
use alloy_sol_types::SolType;
use alloy_primitives::Address;
use reqwest;
use serde::{Deserialize, Serialize, Deserializer};
use sp1_sdk::{include_elf, ProverClient, SP1Stdin, setup_logger, HashableKey, Prover};
use fibonacci_lib::PublicValuesBtcHoldings;
use sha3::{Digest, Keccak256};
use anyhow::Result;
use hex;

// SP1 ELF binary
pub const DOGE_DEPOSIT_ELF: &[u8] = include_elf!("doge-deposit-program");

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DogeDepositRequest {
    tx_hash: String,
    wallet_address: String,
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
    hex: Option<String>,
    locktime: u64,
    #[serde(default)]
    outputs: Vec<TatumOutput>,
    #[serde(rename = "vin", default)]
    inputs: Vec<TatumInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumInput {
    #[serde(default)]
    prevout: Option<TatumPrevout>,
    #[serde(default)]
    script_sig: TatumScriptSig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumPrevout {
    hash: String,
    #[serde(alias = "vout")]
    index: u32,
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
    address: String,
    #[serde(deserialize_with = "deserialize_value")]
    value: String,
}

fn deserialize_value<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
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

fn pubkey_to_address(pubkey: &[u8]) -> Result<Address> {
    let hash = Keccak256::digest(pubkey);
    Ok(Address::from_slice(&hash[12..32]))
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
async fn prove_doge_deposit(
    req: web::Json<DogeDepositRequest>,
) -> Result<impl Responder, ActixError> {
    println!("Received Dogecoin deposit proof request: {:?}", req);

    // Fetch transaction details
    let tx = match fetch_doge_transaction(&req.tx_hash).await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to fetch transaction: {:?}", e);
            return Ok(HttpResponse::InternalServerError().body(format!("Transaction fetch failed: {}", e)));
        }
    };

    // Extract sender pubkey from scriptSig
    let sender_pubkey = if let Some(input) = tx.inputs.first() {
        let parts: Vec<&str> = input.script_sig.asm.split_whitespace().collect();
        if parts.is_empty() {
            eprintln!("Empty scriptSig");
            return Ok(HttpResponse::BadRequest().body("Empty scriptSig"));
        }

        // The public key is the last element in scriptSig.asm
        let pubkey_hex = parts.last().ok_or_else(|| {
            eprintln!("No public key found in scriptSig: {:?}", input.script_sig.asm);
            actix_web::error::ErrorBadRequest("No public key found in scriptSig")
        })?;

        match hex::decode(pubkey_hex) {
            Ok(bytes) => {
                if bytes.len() != 33 && bytes.len() != 65 {
                    eprintln!("Invalid public key length: {} bytes", bytes.len());
                    return Ok(HttpResponse::BadRequest().body("Public key must be 33 or 65 bytes"));
                }
                bytes
            }
            Err(e) => {
                eprintln!("Invalid public key hex: {:?} (hex = {:?})", e, pubkey_hex);
                return Ok(HttpResponse::BadRequest().body("Invalid public key in scriptSig"));
            }
        }
    } else {
        return Ok(HttpResponse::BadRequest().body("No inputs found in transaction"));
    };

    let sender_address = match pubkey_to_address(&sender_pubkey) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Address derivation failed: {:?}", e);
            return Ok(HttpResponse::InternalServerError().body("Failed to derive address"));
        }
    };

    // Calculate deposit amount to the defined wallet
    let deposit_amount: u64 = tx.outputs
        .iter()
        .filter(|output| output.address == req.wallet_address)
        .map(|output| {
            let value_doge: f64 = output.value.parse().unwrap_or(0.0);
            (value_doge * 100_000_000.0) as u64 // Convert DOGE to satoshis
        })
        .sum();

    if deposit_amount == 0 {
        return Ok(HttpResponse::BadRequest().body("No deposit found to the specified wallet address"));
    }

    // Convert wallet address to 20-byte Address
    let wallet_address_bytes = if req.wallet_address.starts_with("0x") {
        match hex::decode(&req.wallet_address[2..]) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("Invalid wallet hex: {:?}", e);
                return Ok(HttpResponse::BadRequest().body("Invalid wallet address"));
            }
        }
    } else {
        return Ok(HttpResponse::BadRequest().body("Wallet address must start with 0x"));
    };

    if wallet_address_bytes.len() != 20 {
        return Ok(HttpResponse::BadRequest().body("Wallet address must be a 20-byte Ethereum address"));
    }

    let wallet_address = Address::from_slice(&wallet_address_bytes);

    // Proving
    let client = ProverClient::builder().network().build();
    let (pk, vk) = client.setup(DOGE_DEPOSIT_ELF);

    let mut stdin = SP1Stdin::new();
    stdin.write(&sender_address.as_slice());
    stdin.write(&deposit_amount);
    stdin.write(&wallet_address.as_slice());

    let proof_result = match req.proof_system.as_str() {
        "plonk" => client.prove(&pk, &stdin).plonk().run(),
        "groth16" => client.prove(&pk, &stdin).groth16().run(),
        _ => return Ok(HttpResponse::BadRequest().body("Invalid proof system")),
    };

    let (proof, vk) = match proof_result {
        Ok(proof) => (proof, vk),
        Err(e) => {
            eprintln!("Proof generation error: {:?}", e);
            return Ok(HttpResponse::InternalServerError().body("Proof generation failed"));
        }
    };

    let public_bytes = proof.public_values.as_slice();
    let public_values = match PublicValuesBtcHoldings::abi_decode(public_bytes) {
        Ok(val) => val,
        Err(e) => {
            eprintln!("Public value decode failed: {:?}", e);
            return Ok(HttpResponse::InternalServerError().body("Failed to decode public values"));
        }
    };

    Ok(HttpResponse::Ok().json(DogeDepositResponse {
        sender_address: format!("0x{}", hex::encode(sender_address)),
        deposit_amount: public_values.deposit_amount,
        wallet_address: format!("0x{}", hex::encode(wallet_address)),
        vkey: vk.bytes32(),
        public_values: format!("0x{}", hex::encode(public_bytes)),
        proof: format!("0x{}", hex::encode(proof.bytes())),
    }))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    setup_logger();
    println!("Starting Dogecoin Deposit SP1 proof server on http://localhost:3005");

    HttpServer::new(|| App::new().service(prove_doge_deposit))
        .workers(4)
        .bind(("0.0.0.0", 3005))?
        .run()
        .await
}