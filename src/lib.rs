pub mod bridge;
pub mod client;
pub mod configs;
pub mod encoding;
pub mod rpc;
pub mod workers;

pub type WireTransaction = Vec<u8>;

pub const DEFAULT_RPC_ADDR: &str = "http://127.0.0.1:8899";
pub const DEFAULT_WS_ADDR: &str = "ws://127.0.0.1:8900";
pub const DEFAULT_TX_MAX_RETRIES: u16 = 2;
pub const DEFAULT_TX_RETRY_BATCH_SIZE: usize = 20;

#[cfg(test)]
mod test_utils {
    use std::time::Duration;

    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_sdk::{
        message::Message, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair,
        signer::Signer, system_instruction, transaction::Transaction,
    };
    use tokio::time::sleep;

    pub async fn create_transaction(rpc_client: &RpcClient) -> Transaction {
        let payer = Keypair::new();
        let to_pubkey = Pubkey::new_unique();

        // request airdrop to payer
        rpc_client
            .request_airdrop(&payer.pubkey(), LAMPORTS_PER_SOL * 2)
            .await
            .unwrap();

        sleep(Duration::from_secs(2)).await;

        // transfer instruction
        let instruction =
            system_instruction::transfer(&payer.pubkey(), &to_pubkey, LAMPORTS_PER_SOL);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));

        let blockhash = rpc_client.get_latest_blockhash().await.unwrap();

        Transaction::new(&[&payer], message, blockhash)
    }
}
