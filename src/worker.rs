use std::collections::HashSet;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use solana_client::{
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    rpc_response,
};

use solana_sdk::signature::Signature;
use solana_sdk::transaction;
use solana_transaction_status::TransactionConfirmationStatus;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::RawTransaction;

pub type ConfirmationStatus = bool;

/// Retry transactions to a maximum of `u16` times, keep a track of confirmed transactions
#[derive(Clone)]
pub struct LightWorker {
    /// Transactions queue for retrying
    enqued_txs: Arc<RwLock<HashMap<Signature, (RawTransaction, u16)>>>,
    /// Transactions confirmed
    confirmed_txs: Arc<RwLock<HashSet<Signature>>>,
    /// Rpc Client
    rpc_client: Arc<RpcClient>,
    /// Tpu Client
    tpu_client: Arc<TpuClient>,
}

impl LightWorker {
    pub fn new(rpc_client: Arc<RpcClient>, tpu_client: Arc<TpuClient>) -> Self {
        Self {
            enqued_txs: Default::default(),
            confirmed_txs: Default::default(),
            rpc_client,
            tpu_client,
        }
    }
    /// en-queue transaction if it doesn't already exist
    pub async fn enqnue_tx(&self, sig: Signature, raw_tx: RawTransaction, max_retries: u16) {
        self.enqued_txs
            .write()
            .await
            .entry(sig)
            .or_insert((raw_tx, max_retries));
    }

    /// check if tx is in the confirmed cache
    ///
    /// ## Return
    ///
    /// None if transaction is un-confirmed
    /// Some(Err) in case of transaction failure
    /// Some(Ok(())) if tx is confirmed without failure
    pub async fn confirm_tx(&self, sig: Signature) -> Option<transaction::Result<()>> {
        if self.confirmed_txs.read().await.contains(&sig) {
            Some(Ok(()))
        } else {
            let res = self.rpc_client.get_signature_status(&sig).await.unwrap();
            if res.is_some() {
                self.enqued_txs.write().await.remove(&sig);
                self.confirmed_txs.write().await.insert(sig);
            }
            res
        }
    }

    /// retry enqued_tx(s)
    pub async fn retry_txs(&self) {
        if self.has_no_work().await {
            return;
        }

        let mut enqued_tx = self.enqued_txs.write().await;

        let mut tx_batch = Vec::with_capacity(enqued_tx.len());
        let mut stale_txs = vec![];

        for (sig, (tx, retries)) in enqued_tx.iter_mut() {
            tx_batch.push(tx.clone());
            let Some(retries_left) = retries.checked_sub(1) else {
                stale_txs.push(sig.to_owned());
                continue;
            };
            *retries = retries_left;
        }

        // remove stale tx(s)
        for stale_tx in stale_txs {
            enqued_tx.remove(&stale_tx);
        }

        self.tpu_client
            .try_send_wire_transaction_batch(tx_batch)
            .await
            .unwrap();
    }

    /// confirm enqued_tx(s)
    pub async fn confirm_txs(&mut self) {
        if self.has_no_work().await {
            return;
        }

        let mut enqued_txs = self.enqued_txs.write().await;
        let mut confirmed_txs = self.confirmed_txs.write().await;

        let mut signatures: Vec<Signature> = enqued_txs.keys().cloned().collect();

        let rpc_response::Response { context: _, value } = self
            .rpc_client
            .get_signature_statuses(&signatures)
            .await
            .unwrap();

        let mut signatures = signatures.iter_mut();

        for tx_status in value {
            let signature = *signatures.next().unwrap();
            let Some(tx_status) = tx_status else {
                enqued_txs.remove(&signature);
                continue;
            };

            match tx_status.confirmation_status() {
                TransactionConfirmationStatus::Processed => (),
                _status => {
                    enqued_txs.remove(&signature);
                    confirmed_txs.insert(signature);
                }
            };
        }
    }

    /// check if any transaction is pending, still enqueued
    pub async fn has_no_work(&self) -> bool {
        self.enqued_txs.read().await.is_empty()
    }

    /// retry and confirm transactions every 800ms (avg time to confirm tx)
    pub fn execute(mut self) -> JoinHandle<()> {
        let mut interval = tokio::time::interval(Duration::from_millis(800));
        tokio::spawn(async move {
            loop {
                println!("retrying tx");
                self.retry_txs().await;

                interval.tick().await;

                println!("confirming txs");
                self.confirm_txs().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_client::nonblocking::tpu_client::TpuClient;
    use solana_sdk::message::Message;
    use solana_sdk::native_token::LAMPORTS_PER_SOL;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::Keypair;
    use solana_sdk::signer::Signer;
    use solana_sdk::system_instruction;
    use solana_sdk::transaction::Transaction;

    use crate::encoding::BinaryEncoding;

    use super::LightWorker;

    const RPC_ADDR: &str = "http://127.0.0.1:8899";
    const WS_ADDR: &str = "ws://127.0.0.1:8900";

    #[tokio::test]
    async fn worker_test() {
        let rpc_client = Arc::new(RpcClient::new(RPC_ADDR.to_string()));
        let tpu_client = Arc::new(
            TpuClient::new(rpc_client.clone(), WS_ADDR, Default::default())
                .await
                .unwrap(),
        );

        let worker = LightWorker::new(rpc_client.clone(), tpu_client);

        let payer = Keypair::new();

        rpc_client
            .request_airdrop(&payer.pubkey(), LAMPORTS_PER_SOL * 2)
            .await
            .unwrap();

        std::thread::sleep(Duration::from_secs(2));

        let to_pubkey = Pubkey::new_unique();
        let instruction =
            system_instruction::transfer(&payer.pubkey(), &to_pubkey, LAMPORTS_PER_SOL);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));

        let blockhash = rpc_client.get_latest_blockhash().await.unwrap();

        let tx = Transaction::new(&[&payer], message, blockhash);
        let sig = tx.signatures[0];
        let encoded_signature = BinaryEncoding::Base58.encode(sig);

        let tx = BinaryEncoding::Base58.encode(bincode::serialize(&tx).unwrap());

        worker.enqnue_tx(sig, tx.as_bytes().to_vec(), 5).await;

        assert!(worker.confirm_tx(sig).await.is_some());
    }
}
