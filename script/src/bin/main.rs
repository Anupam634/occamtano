// use axum::{
//     extract::Json,
//     response::IntoResponse,
//     routing::post,
//     Router,
// };
// use fibonacci_lib::{calculate_icr, calculate_liquidation_threshold, real_time_ltv};
// use serde::{Deserialize, Serialize};
// use sp1_sdk::{include_elf, utils::setup_logger, ProverClient, SP1Stdin};
// use std::{fs::File, io::Write, net::SocketAddr};
// use reqwest;
// use anyhow::{Context, Result};

// pub const FIBONACCI_ELF: &[u8] = include_elf!("fibonacci-program");

// #[derive(Debug, Deserialize)]
// pub struct ProofRequest {
//     pub n: u32,
//     pub collateral_amount: u32,
//     pub debt_amount: u32,
//     pub usbd_loan: u32,
//     pub btc_balance: u32,
// }

// #[derive(Debug, Serialize)]
// pub struct ProofResponse {
//     pub icr: u32,
//     pub liquidation_threshold: u32,
//     pub real_time_ltv: u32,
//     pub proof_fixture: String,
// }

// async fn prove_icr(Json(req): Json<ProofRequest>) -> impl IntoResponse {
//     match generate_proof(req).await {
//         Ok(resp) => axum::Json(resp).into_response(),
//         Err(e) => {
//             eprintln!("Error generating proof: {}", e); // Log the error to the console
//             (
//                 axum::http::StatusCode::INTERNAL_SERVER_ERROR,
//                 format!("Error: {}", e),
//             )
//                 .into_response()
//         }
//     }
// }

// async fn fetch_btc_price() -> Result<u32> {
//     let url = "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd";
//     let resp = reqwest::get(url).await.context("Failed to fetch BTC price")?;
//     let resp: serde_json::Value = resp.json().await.context("Failed to parse BTC price response")?;
//     Ok(resp["bitcoin"]["usd"]
//         .as_f64()
//         .unwrap_or(0.0) as u32)
// }

// async fn generate_proof(req: ProofRequest) -> Result<ProofResponse> {
//     let btc_price_usd = fetch_btc_price().await?;

//     let mut stdin = SP1Stdin::new();
//     stdin.write(&req.n);
//     stdin.write(&req.collateral_amount);
//     stdin.write(&req.debt_amount);
//     stdin.write(&btc_price_usd);
//     stdin.write(&req.usbd_loan);
//     stdin.write(&req.btc_balance);

//     let client = ProverClient::from_env();
//     let (pk, vk) = client.setup(FIBONACCI_ELF);

//     eprintln!("Proving with inputs: {:?}", req);

//     match client.prove(&pk, &stdin).run() {
//         Ok(proof) => {
//             eprintln!("Proof generated successfully.");

//             match client.verify(&proof, &vk) {
//                 Ok(_) => {
//                     eprintln!("Proof verified successfully.");
//                 }
//                 Err(e) => {
//                     eprintln!("Proof verification failed: {}", e);
//                     return Err(anyhow::anyhow!("Proof verification failed: {}", e).into());
//                 }
//             }

//             let proof_json = serde_json::to_string_pretty(&proof)?;
//             let mut file = File::create("proof_fixture.json").context("Failed to create file")?;
//             file.write_all(proof_json.as_bytes())
//                 .context("Failed to write proof to file")?;

//             Ok(ProofResponse {
//                 icr: calculate_icr(req.collateral_amount, req.debt_amount, btc_price_usd).0,
//                 liquidation_threshold: calculate_liquidation_threshold(
//                     req.collateral_amount,
//                     req.btc_balance,
//                     req.collateral_amount / req.debt_amount,
//                 ),
//                 real_time_ltv: real_time_ltv(req.usbd_loan, req.btc_balance, req.btc_balance),
//                 proof_fixture: proof_json,
//             })
//         }
//         Err(e) => {
//             eprintln!("Proof generation failed: {}", e);
//             Err(anyhow::anyhow!("Proof generation failed: {}", e).into())
//         }
//     }
// }

// #[tokio::main]
// async fn main() {
//     setup_logger();

//     let app = Router::new().route("/prove_icr", post(prove_icr));
//     let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
//     println!("Listening on http://{}", addr);
//     axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app)
//         .await
//         .unwrap();
// }