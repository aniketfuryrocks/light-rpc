use crate::{
    block_listenser::BlockListener,
    configs::SendTransactionConfig,
    encoding::BinaryEncoding,
    rpc::{
        ConfirmTransactionParams, JsonRpcError, JsonRpcReq, JsonRpcRes, RpcMethod,
        SendTransactionParams,
    },
    worker::LightWorker,
};

use std::{net::ToSocketAddrs, str::FromStr, sync::Arc};

use actix_web::{web, App, HttpServer, Responder};
use reqwest::Url;

use solana_client::{
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    rpc_response::RpcVersionInfo,
};
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};

/// A bridge between clients and tpu
pub struct LightBridge {
    pub tpu_client: Arc<TpuClient>,
    pub rpc_url: Url,
    pub worker: LightWorker,
    pub block_listner: BlockListener,
}

impl LightBridge {
    pub async fn new(rpc_url: reqwest::Url, ws_addr: &str) -> anyhow::Result<Self> {
        let rpc_client = Arc::new(RpcClient::new(rpc_url.to_string()));

        let tpu_client =
            Arc::new(TpuClient::new(rpc_client.clone(), ws_addr, Default::default()).await?);

        let block_listner = BlockListener::new(rpc_client.clone(), ws_addr).await?;

        Ok(Self {
            worker: LightWorker::new(tpu_client.clone(), block_listner.clone()),
            block_listner,
            rpc_url,
            tpu_client,
        })
    }

    pub async fn send_transaction(
        &self,
        SendTransactionParams(
            tx,
            SendTransactionConfig {
                skip_preflight: _,       //TODO:
                preflight_commitment: _, //TODO:
                encoding,
                max_retries,
                min_context_slot: _, //TODO:
            },
        ): SendTransactionParams,
    ) -> Result<String, JsonRpcError> {
        let raw_tx = encoding.decode(tx)?;

        let sig = bincode::deserialize::<VersionedTransaction>(&raw_tx)?.signatures[0];

        self.tpu_client.send_wire_transaction(raw_tx.clone()).await;

        self.worker
            .enqnueue_tx(sig, raw_tx, max_retries.unwrap_or(1))
            .await;

        Ok(BinaryEncoding::Base58.encode(sig))
    }

    pub async fn confirm_transaction(
        &self,
        ConfirmTransactionParams(sig, _): ConfirmTransactionParams,
    ) -> Result<bool, JsonRpcError> {
        let sig = Signature::from_str(&sig)?;

        Ok(self.block_listner.confirm_tx(sig).await.is_some())
    }

    pub fn get_version(&self) -> RpcVersionInfo {
        let version = solana_version::Version::default();
        RpcVersionInfo {
            solana_core: version.to_string(),
            feature_set: Some(version.feature_set),
        }
    }

    /// Serialize params and execute the specified method
    pub async fn execute_rpc_request(
        &self,
        JsonRpcReq { method, params }: JsonRpcReq,
    ) -> Result<serde_json::Value, JsonRpcError> {
        match method {
            RpcMethod::SendTransaction => Ok(self
                .send_transaction(serde_json::from_value(params)?)
                .await?
                .into()),
            RpcMethod::ConfirmTransaction => Ok(self
                .confirm_transaction(serde_json::from_value(params)?)
                .await?
                .into()),
            RpcMethod::GetVersion => Ok(serde_json::to_value(self.get_version()).unwrap()),
            RpcMethod::Other => unreachable!("Other Rpc Methods should be handled externally"),
        }
    }

    /// List for `JsonRpc` requests
    pub async fn start_server(self, addr: impl ToSocketAddrs) -> anyhow::Result<()> {
        let this = Arc::new(self);
        let worker = this.worker.clone().execute();
        let block_listenser = this.block_listner.clone().listen();

        let json_cfg = web::JsonConfig::default().error_handler(|err, req| {
            let err = JsonRpcRes::Err(serde_json::Value::String(format!("{err}")))
                .respond_to(req)
                .into_body();
            actix_web::error::ErrorBadRequest(err)
        });

        let server = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(this.clone()))
                .app_data(json_cfg.clone())
                .route("/", web::post().to(Self::rpc_route))
        })
        .bind(addr)?
        .run();

        let (res1, res2, res3) = tokio::join!(server, worker, block_listenser);
        res1?;
        res2?;
        res3??;
        Ok(())
    }

    async fn rpc_route(body: bytes::Bytes, state: web::Data<Arc<LightBridge>>) -> JsonRpcRes {
        let json_rpc_req = match serde_json::from_slice::<JsonRpcReq>(&body) {
            Ok(json_rpc_req) => json_rpc_req,
            Err(err) => return JsonRpcError::SerdeError(err).into(),
        };

        if let RpcMethod::Other = json_rpc_req.method {
            let res = reqwest::Client::new()
                .post(state.rpc_url.clone())
                .body(body)
                .header("Content-Type", "application/json")
                .send()
                .await
                .unwrap();

            JsonRpcRes::Raw {
                status: res.status().as_u16(),
                body: res.text().await.unwrap(),
            }
        } else {
            state
                .execute_rpc_request(json_rpc_req)
                .await
                .try_into()
                .unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use reqwest::Url;
    use solana_client::rpc_response::RpcVersionInfo;
    use solana_sdk::{
        message::Message, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair,
        signer::Signer, system_instruction, transaction::Transaction,
    };

    use crate::rpc::SendTransactionParams;
    use crate::{bridge::LightBridge, encoding::BinaryEncoding};

    const RPC_ADDR: &str = "http://127.0.0.1:8899";
    const WS_ADDR: &str = "ws://127.0.0.1:8900";

    #[tokio::test]
    async fn get_version() {
        let light_bridge = LightBridge::new(Url::from_str(RPC_ADDR).unwrap(), WS_ADDR)
            .await
            .unwrap();

        let RpcVersionInfo {
            solana_core,
            feature_set,
        } = light_bridge.get_version();
        let version_crate = solana_version::Version::default();

        assert_eq!(solana_core, version_crate.to_string());
        assert_eq!(feature_set.unwrap(), version_crate.feature_set);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_send_transaction() {
        let light_bridge = LightBridge::new(Url::from_str(RPC_ADDR).unwrap(), WS_ADDR)
            .await
            .unwrap();

        let payer = Keypair::new();

        light_bridge
            .tpu_client
            .rpc_client()
            .request_airdrop(&payer.pubkey(), LAMPORTS_PER_SOL * 2)
            .await
            .unwrap();

        std::thread::sleep(Duration::from_secs(2));

        let to_pubkey = Pubkey::new_unique();
        let instruction =
            system_instruction::transfer(&payer.pubkey(), &to_pubkey, LAMPORTS_PER_SOL);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));

        let blockhash = light_bridge
            .tpu_client
            .rpc_client()
            .get_latest_blockhash()
            .await
            .unwrap();

        let tx = Transaction::new(&[&payer], message, blockhash);
        let signature = tx.signatures[0];
        let encoded_signature = BinaryEncoding::Base58.encode(signature);

        let tx = BinaryEncoding::Base58.encode(bincode::serialize(&tx).unwrap());

        assert_eq!(
            light_bridge
                .send_transaction(SendTransactionParams(tx, Default::default()))
                .await
                .unwrap(),
            encoded_signature
        );

        std::thread::sleep(Duration::from_secs(5));

        let mut passed = false;

        for _ in 0..100 {
            passed = light_bridge
                .tpu_client
                .rpc_client()
                .confirm_transaction(&signature)
                .await
                .unwrap();

            std::thread::sleep(Duration::from_millis(100));
        }

        passed.then_some(()).unwrap();
    }
}
