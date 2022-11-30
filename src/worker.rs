use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Future;
use solana_client::{
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    rpc_response,
};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::TransactionConfirmationStatus;

use crate::RawTransaction;

pub type ConfirmationStatus = bool;

/// Retry transactions to a maximum of `u16` times, keep a track of confirmed transactions
pub struct LightWorker {
    /// Transactions queue for retrying
    enqued_tx: HashMap<Signature, (RawTransaction, u16)>,
    /// Transactions confirmed
    confirmed_tx: HashMap<Signature, TransactionConfirmationStatus>,
    /// Rpc Client
    rpc_client: RpcClient,
    /// Tpu Client
    tpu_client: TpuClient,
}

impl LightWorker {
    /// en-queue transaction if it doesn't already exist
    pub fn enqnue_tx(&mut self, sig: Signature, raw_tx: RawTransaction, max_retries: u16) {
        self.enqued_tx.entry(sig).or_insert((raw_tx, max_retries));
    }

    /// check if tx is in the confirmed cache
    pub async fn confirm_tx(&self, sig: &Signature) -> Option<TransactionConfirmationStatus> {
        let Some(status) = self.confirmed_tx.get(sig) else {
            let k = self.rpc_client.get_signature_status_with_commitment(sig, CommitmentConfig::confirmed()).await.unwrap();
            todo!()
        };

        Some(status.to_owned())
    }

    /// retry enqued_tx(s)
    pub async fn retry_txs(&mut self) {
        let mut tx_batch = Vec::with_capacity(self.enqued_tx.len());
        let mut stale_txs = vec![];

        for (sig, (tx, retries)) in self.enqued_tx.iter_mut() {
            tx_batch.push(tx.clone());
            let Some(retries_left) = retries.checked_sub(1) else {
                stale_txs.push(sig.to_owned());
                continue;
            };
            *retries = retries_left;
        }

        // remove stale tx(s)
        for stale_tx in stale_txs {
            self.enqued_tx.remove(&stale_tx);
        }

        self.tpu_client
            .try_send_wire_transaction_batch(tx_batch)
            .await
            .unwrap();
    }

    /// confirm enqued_tx(s)
    pub async fn confirm_txs(&mut self) {
        let mut signatures: Vec<Signature> = self.enqued_tx.keys().cloned().collect();

        let rpc_response::Response { context: _, value } = self
            .rpc_client
            .get_signature_statuses(&signatures)
            .await
            .unwrap();

        let mut signatures = signatures.iter_mut();

        for tx_status in value {
            let signature = *signatures.next().unwrap();
            let Some(tx_status) = tx_status else {
                todo!("remove invalid signatures")
            };

            match tx_status.confirmation_status() {
                TransactionConfirmationStatus::Processed => (),
                status => {
                    self.enqued_tx.remove(&signature);
                    self.confirmed_tx.insert(signature, status);
                }
            };
        }
    }
}

impl Future for LightWorker {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(())
    }
}
