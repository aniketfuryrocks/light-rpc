mod block_listenser;
mod tx_sender;

pub use block_listenser::*;
pub use tx_sender::*;

#[cfg(test)]
mod worker_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::encoding::BinaryEncoding;
    use crate::test_utils::create_transaction;
    use crate::{DEFAULT_RPC_ADDR, DEFAULT_WS_ADDR};
    use futures::future::join;
    use solana_client::nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient};

    use super::{BlockListener, TxSender};

    #[tokio::test]
    async fn send_and_confirm_txs() {
        let rpc_client = Arc::new(RpcClient::new(DEFAULT_RPC_ADDR.to_string()));
        let tpu_client = Arc::new(
            TpuClient::new(rpc_client.clone(), DEFAULT_WS_ADDR, Default::default())
                .await
                .unwrap(),
        );

        let block_listener = BlockListener::new(rpc_client.clone(), DEFAULT_WS_ADDR)
            .await
            .unwrap();

        let tx_sender = TxSender::new(tpu_client, block_listener.clone());

        let services = join(block_listener.clone().listen(), tx_sender.clone().execute());

        let confirm = tokio::spawn(async move {
            let tx = create_transaction(&rpc_client).await;
            let sig = tx.signatures[0];
            let tx = BinaryEncoding::Base58.encode(bincode::serialize(&tx).unwrap());

            tx_sender.enqnueue_tx(sig, tx.as_bytes().to_vec(), 2).await;

            let sig = sig.to_string();

            for _ in 0..2 {
                if block_listener.confirmed_txs.read().await.contains(&sig) {
                    return;
                }

                tokio::time::sleep(Duration::from_millis(800)).await;
            }

            panic!("Tx {sig} not confirmed in 1600ms");
        });

        tokio::select! {
            _ = services => {
                panic!("Services stopped unexpectedly")
            },
            _ = confirm => {}
        }
    }
}
