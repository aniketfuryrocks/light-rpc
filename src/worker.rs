use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use log::{info, warn};

use solana_client::nonblocking::tpu_client::TpuClient;

use solana_sdk::signature::Signature;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::block_listenser::BlockListener;
use crate::WireTransaction;

const RETRY_BATCH_SIZE: usize = 10;

/// Retry transactions to a maximum of `u16` times, keep a track of confirmed transactions
#[derive(Clone)]
pub struct LightWorker {
    /// Transactions queue for retrying
    enqueued_txs: Arc<RwLock<HashMap<Signature, (WireTransaction, u16)>>>,
    /// block_listner
    block_listner: BlockListener,
    /// TpuClient to call the tpu port
    tpu_client: Arc<TpuClient>,
}

impl LightWorker {
    pub fn new(tpu_client: Arc<TpuClient>, block_listner: BlockListener) -> Self {
        Self {
            enqueued_txs: Default::default(),
            block_listner,
            tpu_client,
        }
    }
    /// en-queue transaction if it doesn't already exist
    pub async fn enqnueue_tx(&self, sig: Signature, raw_tx: WireTransaction, max_retries: u16) {
        if !self
            .block_listner
            .confirmed_txs
            .read()
            .await
            .contains(&sig.to_string())
        {
            info!("en-queuing {sig} with max retries {max_retries}");
            self.enqueued_txs
                .write()
                .await
                .insert(sig, (raw_tx, max_retries));

            println!("{:?}", self.enqueued_txs.read().await.len());
        }
    }

    /// retry enqued_tx(s)
    pub async fn retry_txs(&self) {
        if self.has_no_work().await {
            return;
        }

        info!("retrying tx(s)");

        let mut enqued_tx = self.enqueued_txs.write().await;

        let mut tx_batch = Vec::with_capacity(enqued_tx.len() / RETRY_BATCH_SIZE);
        let mut stale_txs = vec![];

        let mut batch_index = 0;

        for (index, (sig, (tx, retries))) in enqued_tx.iter_mut().enumerate() {
            if index % RETRY_BATCH_SIZE == 0 {
                tx_batch.push(Vec::with_capacity(RETRY_BATCH_SIZE));
                batch_index += 1;
            }

            tx_batch[batch_index - 1].push(tx.clone());

            let Some(retries_left) = retries.checked_sub(1) else {
                stale_txs.push(sig.to_owned());
                continue;
            };

            info!("retrying {sig} with {retries_left} retries left");

            *retries = retries_left;
        }

        // remove stale tx(s)
        for stale_tx in stale_txs {
            enqued_tx.remove(&stale_tx);
        }

        for tx_batch in tx_batch {
            if let Err(err) = self
                .tpu_client
                .try_send_wire_transaction_batch(tx_batch)
                .await
            {
                warn!("{err}");
            }
        }
    }

    /// check if any transaction is pending, still enqueued
    pub async fn has_no_work(&self) -> bool {
        self.enqueued_txs.read().await.is_empty()
    }

    /// retry and confirm transactions every 800ms (avg time to confirm tx)
    pub fn execute(self) -> JoinHandle<anyhow::Result<()>> {
        let mut interval = tokio::time::interval(Duration::from_secs(800));

        #[allow(unreachable_code)]
        tokio::spawn(async move {
            loop {
                info!("{} tx(s) en-queued", self.enqueued_txs.read().await.len());
                interval.tick().await;
                self.retry_txs().await;
            }

            // to give the correct type to JoinHandle
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::encoding::BinaryEncoding;
    use crate::test_utils::create_transaction;
    use solana_client::nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient};

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

        let worker = LightWorker::new(tpu_client);
        worker.clone().execute();

        let tx = create_transaction(&rpc_client).await;

        let sig = tx.signatures[0];

        let tx = BinaryEncoding::Base58.encode(bincode::serialize(&tx).unwrap());

        worker.enqnueue_tx(sig, tx.as_bytes().to_vec(), 5).await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        assert!(worker.confirm_tx(sig).await.is_some());
    }
}
