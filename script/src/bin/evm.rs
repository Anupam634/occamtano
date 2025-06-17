use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use alloy_sol_types::SolType;
use reqwest;
use serde::{Deserialize, Serialize};
use sp1_sdk::{include_elf, ProverClient, HashableKey, utils::setup_logger};
use std::error::Error;
use fibonacci_lib::{PublicValuesBtcHoldings, Utxo, BtcHoldingsInput};
use tokio::task;
use anyhow::Result;
use hex;

pub const BTC_HOLDINGS_ELF: &[u8] = include_elf!("btc-holdings-program");

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtcHoldingsRequest {
    btc_address: String,
    org_id: String,
    proof_system: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtcHoldingsResponse {
    total_btc: u64,
    org_hash: String,
    vkey: String,
    public_values: String,
    proof: String,
    verifier_version: String,
}

#[derive(Debug, Deserialize)]
struct BlockstreamUtxo {
    txid: String,
    vout: u32,
    value: u64,
}

#[post("/prove-btc-holdings")]
async fn prove_btc_holdings(req: web::Json<BtcHoldingsRequest>) -> impl Responder {
    println!("Received BTC holdings proof request: {:?}", req);

    let utxos = match fetch_utxos(&req.btc_address).await {
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
        let (pk, vk) = client.setup(BTC_HOLDINGS_ELF);
        let mut stdin = sp1_sdk::SP1Stdin::new();

        let utxos = utxos
            .into_iter()
            .map(|u| {
                let txid = hex::decode(&u.txid).expect("valid txid hex");
                let pubkey = vec![0u8; 33]; // dummy compressed pubkey
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
            signatures: vec![], // No signatures used
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

    let response = BtcHoldingsResponse {
        total_btc: public_values.total_btc,
        org_hash: format!("0x{}", hex::encode(public_values.org_hash)),
        vkey: vk.bytes32(),
        public_values: format!("0x{}", hex::encode(public_bytes)),
        proof: format!("0x{}", hex::encode(proof.bytes())),
        verifier_version: "0x1234abcd".to_string(),
    };

    HttpResponse::Ok().json(response)
}

async fn fetch_utxos(address: &str) -> Result<Vec<BlockstreamUtxo>, Box<dyn Error>> {
    let url = format!("https://blockstream.info/api/address/{}/utxo", address);
    let resp = reqwest::get(&url).await?;
    let utxos: Vec<BlockstreamUtxo> = resp.json().await?;
    Ok(utxos)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    setup_logger();
    println!("Starting BTC Holdings SP1 proof server on http://localhost:8080");

    HttpServer::new(|| App::new().service(prove_btc_holdings))
        .workers(4)
        .bind(("0.0.0.0", 8080))?
        .run()
        .await
}