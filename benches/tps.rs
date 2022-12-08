use std::time::{Duration, Instant};

use light_rpc::client::LightClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;

const RPC_ADDR: &str = "http://127.0.0.1:8890";
const TPU_ADDR: &str = "127.0.0.1:1027";
const WS_ADDR: &str = "ws://127.0.0.1:8900";
const CONNECTION_POOL_SIZE: usize = 1024;

const LAMPORTS_TO_SEND_PER_TX: u64 = 1_000_000;
const PAYER_BANK_BALANCE: u64 = LAMPORTS_PER_SOL * 20;
const NUMBER_OF_TXS: u64 = 1;
const NUMBER_OF_RUNS: u64 = 1;

#[derive(serde::Serialize)]
struct Metrics {
    #[serde(rename = "Number of TX(s)")]
    number_of_transactions: u64,
    #[serde(rename = "Time To Send TX(s) ms")]
    time_to_send_txs: u128,
    #[serde(rename = "Time To Confirm TX(s) ms")]
    time_to_confirm_txs: u128,
    #[serde(rename = "Total duration(ms)")]
    duration: u128,
    #[serde(rename = "TPS")]
    tps: u64,
}

#[tokio::main]
async fn main() {
    let light_client = LightClient(RpcClient::new(RPC_ADDR.to_string()));

    let mut wtr = csv::Writer::from_path("metrics.csv").unwrap();
    let mut metrics = Vec::new();

    let payer = create_and_confirm_new_account_with_funds(&light_client, PAYER_BANK_BALANCE).await;

    // send transactions
    for _ in 0..NUMBER_OF_RUNS {
        let lastest_block_hash = light_client.get_latest_blockhash().await.unwrap();

        //
        // Send TX's
        //

        let send_start_time = Instant::now();

        let signatures = send_funds_to_random_accounts(
            &light_client,
            &payer,
            lastest_block_hash,
            NUMBER_OF_TXS,
            LAMPORTS_TO_SEND_PER_TX,
        )
        .await;

        let time_to_send_txs = send_start_time.elapsed().as_millis();

        //
        // Confirm TX's
        //

        let confirm_start_time = Instant::now();

        confirm_transactions(&light_client, signatures).await;

        let time_to_confirm_txs = confirm_start_time.elapsed().as_millis();

        metrics.push(Metrics {
            number_of_transactions: NUMBER_OF_TXS,
            time_to_send_txs,
            time_to_confirm_txs,
            duration: send_start_time.elapsed().as_millis(),
            tps: (NUMBER_OF_TXS / send_start_time.elapsed().as_secs()),
        });
    }

    for metric in metrics {
        wtr.serialize(metric).unwrap();
    }
}

async fn create_and_confirm_new_account_with_funds(
    light_client: &LightClient,
    funds: u64,
) -> Keypair {
    let payer = Keypair::new();
    let payer_pub_key = payer.pubkey();

    let airdrop_signature = light_client
        .request_airdrop(&payer_pub_key, funds)
        .await
        .expect("error requesting airdrop");

    confirm_transactions(light_client, vec![airdrop_signature.to_string()]).await;

    payer
}

async fn send_funds_to_random_accounts(
    light_client: &LightClient,
    payer: &Keypair,
    latest_blockhash: solana_sdk::hash::Hash,
    number_of_accounts: u64,
    amount_to_send: u64,
) -> Vec<String> {
    let mut transaction_signatures = Vec::with_capacity(number_of_accounts as usize);

    for _ in 0..number_of_accounts {
        let to_pubkey = Pubkey::new_unique();
        let ix = system_instruction::transfer(&payer.pubkey(), &to_pubkey, amount_to_send);

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[payer],
            latest_blockhash,
        );

        let signature = light_client.send_transaction(&tx).await.unwrap();

        transaction_signatures.push(signature.to_string());
    }

    transaction_signatures
}

async fn confirm_transactions(light_client: &LightClient, mut signatures: Vec<String>) {
    let mut signatures_to_retry = Vec::with_capacity(signatures.len());

    loop {
        for signature in signatures {
            if light_client.confirm_transaction(signature.clone()).await {
                eprintln!("confirmed : {signature}");
            } else {
                eprintln!("confirming {}", signature);
                signatures_to_retry.push(signature);
            }
        }

        if signatures_to_retry.is_empty() {
            return;
        }

        signatures = signatures_to_retry.clone();
        signatures_to_retry.clear();

        std::thread::sleep(Duration::from_millis(1000));
    }
}
