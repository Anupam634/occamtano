use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use alloy_sol_types::SolType;
use reqwest;
use serde::{Deserialize, Serialize, Deserializer};
use sp1_sdk::{include_elf, ProverClient, HashableKey, utils::setup_logger};
use fibonacci_lib::{PublicValuesBtcHoldings, Utxo, BtcHoldingsInput};
use tokio::task;
use anyhow::Result;
use hex;

pub const DOGE_HOLDINGS_ELF: &[u8] = include_elf!("btc-holdings-program");

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DogeHoldingsRequest {
    doge_address: String,
    org_id: String,
    proof_system: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DogeHoldingsResponse {
    total_doge: u64,
    org_hash: String,
    vkey: String,
    public_values: String,
    proof: String,
    verifier_version: String,
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
    prevout: TatumPrevout,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumPrevout {
    hash: String,
    #[serde(alias = "vout")]
    index: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TatumOutput {
    address: String,
    #[serde(deserialize_with = "deserialize_value")]
    value: String,
    #[serde(default)]
    n: Option<u32>,
}

#[derive(Debug, Clone)]
struct DogeUtxo {
    txid: String,
    vout: u32,
    value: u64,
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

#[post("/prove-doge-holdings")]
async fn prove_doge_holdings(req: web::Json<DogeHoldingsRequest>) -> impl Responder {
    println!("Received Dogecoin holdings proof request: {:?}", req);

    let utxos = match fetch_doge_utxos(&req.doge_address).await {
        Ok(utxos) => utxos,
        Err(e) => {
            eprintln!("Failed to fetch UTXOs: {:?}", e);
            return HttpResponse::InternalServerError().body(format!("UTXO fetch failed: {}", e));
        }
    };

    if utxos.is_empty() {
        return HttpResponse::BadRequest().body("No UTXOs found for the address");
    }

    let expected_total = utxos.iter().map(|u| u.value).sum::<u64>();
    let org_id = req.org_id.clone();
    let proof_system = req.proof_system.clone();

    let proof_result = task::spawn_blocking(move || {
        let client = ProverClient::from_env();
        let (pk, vk) = client.setup(DOGE_HOLDINGS_ELF);
        let mut stdin = sp1_sdk::SP1Stdin::new();

        let utxos = utxos
            .into_iter()
            .map(|u| {
                let txid = hex::decode(&u.txid).expect("valid txid hex");
                let pubkey = vec![0u8; 33];
                Utxo {
                    txid: txid.try_into().expect("32 bytes"),
                    index: u.vout,
                    amount: u.value,
                    pubkey,
                }
            })
            .collect::<Vec<_>>();

        let input = BtcHoldingsInput {
            utxos,
            signatures: vec![],
            expected_total,
            org_id,
        };

        stdin.write(&input);

        let proof_result = match proof_system.as_str() {
            "plonk" => client.prove(&pk, &stdin).plonk().run(),
            "groth16" => client.prove(&pk, &stdin).groth16().run(),
            _ => return Err(anyhow::anyhow!("Invalid proof system")),
        };
        Ok(proof_result.map(|proof| (proof, vk))?)
    })
    .await;

    let (proof, vk) = match proof_result {
        Ok(Ok((proof, vk))) => (proof, vk),
        Ok(Err(e)) => {
            eprintln!("Proof generation failed: {:?}", e);
            return HttpResponse::InternalServerError().body(format!("Proof generation failed: {}", e));
        }
        Err(e) => {
            eprintln!("Proof generation task failed: {:?}", e);
            return HttpResponse::InternalServerError().body("Proof generation task failed");
        }
    };

    let public_bytes = proof.public_values.as_slice();
    let public_values = match PublicValuesBtcHoldings::abi_decode(public_bytes) {
        Ok(val) => val,
        Err(e) => {
            eprintln!("Decoding public values failed: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to decode public values");
        }
    };

    let response = DogeHoldingsResponse {
        total_doge: public_values.total_btc,
        org_hash: format!("0x{}", hex::encode(public_values.org_hash)),
        vkey: vk.bytes32(),
        public_values: format!("0x{}", hex::encode(public_bytes)),
        proof: format!("0x{}", hex::encode(proof.bytes())),
        verifier_version: "0x1234abcd".to_string(),
    };

    HttpResponse::Ok().json(response)
}

async fn fetch_doge_utxos(address: &str) -> Result<Vec<DogeUtxo>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut offset = 0;
    let mut all_transactions = vec![];
    let page_size = 50;

    loop {
        let url = format!(
            "https://api.tatum.io/v3/dogecoin/transaction/address/{}?pageSize={}&offset={}",
            address, page_size, offset
        );
        let resp = client
            .get(&url)
            .header("x-api-key", "t-67ae0be674c77aa851dd5cce-bd0e33fb85f646a1931d9d0a")
            .send()
            .await?;

        let body = resp.text().await?;
        let txs: Vec<TatumTransaction> = serde_json::from_str(&body)?;

        if txs.is_empty() {
            break;
        }

        all_transactions.extend(txs);
        offset += page_size;
    }

    let mut cltv_utxos: Vec<DogeUtxo> = Vec::new();

    for tx in &all_transactions {
        for (i, output) in tx.outputs.iter().enumerate() {
            if output.address != address && tx.locktime > 0 {
                let value_doge: f64 = output.value.parse().unwrap_or(0.0);
                let value_sats = (value_doge * 100_000_000.0) as u64;

                cltv_utxos.push(DogeUtxo {
                    txid: tx.hash.clone(),
                    vout: output.n.unwrap_or(i as u32),
                    value: value_sats,
                });
            }
        }
    }

    Ok(cltv_utxos)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    setup_logger();
    println!("Starting Dogecoin Holdings SP1 proof server on http://localhost:3001");

    HttpServer::new(|| App::new().service(prove_doge_holdings))
        .workers(4)
        .bind(("0.0.0.0", 3001))?
        .run()
        .await
}
