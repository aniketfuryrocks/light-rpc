pub mod bridge;
pub mod client;
pub mod configs;
pub mod encoding;
pub mod rpc;
pub mod worker;

pub type WireTransaction = Vec<u8>;

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
