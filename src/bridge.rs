use crate::configs::SendTransactionConfig;
use crate::encoding::BinaryEncoding;
use crate::rpc::{
    ConfirmTransactionParams, JsonRpcError, JsonRpcReq, JsonRpcRes, RpcMethod,
    SendTransactionParams,
};
use actix_web::{web, App, HttpServer, Responder};

use reqwest::Url;
use solana_client::rpc_client::RpcClient;

use solana_client::rpc_response::RpcVersionInfo;
use solana_client::tpu_client::{TpuClient, TpuSenderError};
use solana_client::{connection_cache::ConnectionCache, tpu_connection::TpuConnection};

use solana_sdk::transaction::Transaction;

use std::time::Duration;
use std::{
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

/// A bridge between clients and tpu
pub struct LightBridge {
    pub connection_cache: Arc<ConnectionCache>,
    pub tpu_client: TpuClient,
    pub rpc_url: Url,
    pub tpu_addr: SocketAddr,
}

impl LightBridge {
    pub fn new(
        rpc_url: reqwest::Url,
        tpu_addr: SocketAddr,
        ws_addr: &str,
        connection_pool_size: usize,
    ) -> Result<Self, TpuSenderError> {
        let rpc_client = Arc::new(RpcClient::new(&rpc_url));
        let connection_cache = Arc::new(ConnectionCache::new(connection_pool_size));

        let tpu_client = TpuClient::new_with_connection_cache(
            rpc_client,
            ws_addr,
            Default::default(),
            connection_cache.clone(),
        )?;

        Ok(Self {
            rpc_url,
            tpu_client,
            tpu_addr,
            connection_cache,
        })
    }

    pub fn send_transaction(
        &self,
        SendTransactionParams(
            transaction,
            SendTransactionConfig {
                skip_preflight: _,       //TODO:
                preflight_commitment: _, //TODO:
                encoding,
                max_retries,
                min_context_slot: _, //TODO:
            },
        ): SendTransactionParams,
    ) -> Result<String, JsonRpcError> {
        let wire_transaction = encoding.decode(transaction)?;

        let signature = bincode::deserialize::<Transaction>(&wire_transaction)?.signatures[0];
        let signature = BinaryEncoding::Base58.encode(signature);

        let conn = self.connection_cache.get_connection(&self.tpu_addr);
        conn.send_wire_transaction_async(wire_transaction.clone())?;

        match max_retries.unwrap_or(5) {
            0 => (),
            max_retries => {
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_millis(100));

                    for _ in 0..max_retries {
                        interval.tick().await;

                        if let Err(_err) =
                            conn.send_wire_transaction_async(wire_transaction.clone())
                        {
                            break;
                        }
                    }
                });
            }
        }

        Ok(signature)
    }

    pub fn confirm_transaction(
        &self,
        ConfirmTransactionParams(_signature, _commitment_cfg): ConfirmTransactionParams,
    ) -> Result<String, JsonRpcError> {
        todo!()
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
                .send_transaction(serde_json::from_value(params).unwrap())?
                .into()),
            RpcMethod::ConfirmTransaction => Ok(self
                .confirm_transaction(serde_json::from_value(params).unwrap())?
                .into()),
            RpcMethod::GetVersion => Ok(serde_json::to_value(self.get_version()).unwrap()),
            RpcMethod::Other => unreachable!(),
        }
    }

    /// List for `JsonRpc` requests
    pub async fn start_server(self, addr: impl ToSocketAddrs) -> Result<(), std::io::Error> {
        let bridge = Arc::new(self);

        let json_cfg = web::JsonConfig::default().error_handler(|err, req| {
            let err = JsonRpcRes::Err(serde_json::Value::String(format!("{err}")))
                .respond_to(req)
                .into_body();
            actix_web::error::ErrorBadRequest(err)
        });

        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(bridge.clone()))
                .app_data(json_cfg.clone())
                .route("/", web::post().to(Self::rpc_route))
        })
        .bind(addr)?
        .run()
        .await
    }

    async fn rpc_route(body: bytes::Bytes, state: web::Data<Arc<LightBridge>>) -> JsonRpcRes {
        let Ok(json_rpc_req) = serde_json::from_slice::<JsonRpcReq>(&body) else {
            todo!()
        };

        if let RpcMethod::Other = json_rpc_req.method {
            println!("{:?}", json_rpc_req);
            let res = reqwest::Client::new()
                .post(state.rpc_url.clone())
                .body(body)
                .header("Content-Type", "application/json")
                .send()
                .await
                .unwrap();

            println!("{}", res.status());
            println!("{:?}", res.json::<serde_json::Value>().await.unwrap());
            todo!()
        }

        state
            .execute_rpc_request(json_rpc_req)
            .await
            .try_into()
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use solana_client::rpc_response::RpcVersionInfo;
    use solana_sdk::{
        message::Message, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair,
        signer::Signer, system_instruction, transaction::Transaction,
    };

    use crate::{bridge::LightBridge, encoding::BinaryEncoding};

    const RPC_ADDR: &str = "127.0.0.1:8899";
    const TPU_ADDR: &str = "127.0.0.1:1027";
    const CONNECTION_POOL_SIZE: usize = 1;

    #[test]
    fn get_version() {
        let light_bridge = LightBridge::new(
            RPC_ADDR.parse().unwrap(),
            TPU_ADDR.parse().unwrap(),
            CONNECTION_POOL_SIZE,
        );

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
        let light_bridge = LightBridge::new(
            RPC_ADDR.parse().unwrap(),
            TPU_ADDR.parse().unwrap(),
            CONNECTION_POOL_SIZE,
        );

        let payer = Keypair::new();
        light_bridge
            .thin_client
            .rpc_client()
            .request_airdrop(&payer.pubkey(), LAMPORTS_PER_SOL * 2)
            .unwrap();

        std::thread::sleep(Duration::from_secs(2));

        let to_pubkey = Pubkey::new_unique();
        let instruction =
            system_instruction::transfer(&payer.pubkey(), &to_pubkey, LAMPORTS_PER_SOL);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));

        let blockhash = light_bridge
            .thin_client
            .rpc_client()
            .get_latest_blockhash()
            .unwrap();

        let tx = Transaction::new(&[&payer], message, blockhash);
        let signature = tx.signatures[0];
        let encoded_signature = BinaryEncoding::Base58.encode(signature);

        let tx = BinaryEncoding::Base58.encode(bincode::serialize(&tx).unwrap());

        assert_eq!(
            light_bridge
                .send_transaction(tx, Default::default())
                .unwrap(),
            encoded_signature
        );

        std::thread::sleep(Duration::from_secs(5));

        let mut passed = false;

        for _ in 0..100 {
            passed = light_bridge
                .thin_client
                .rpc_client()
                .confirm_transaction(&signature)
                .unwrap();

            std::thread::sleep(Duration::from_millis(100));
        }

        passed.then_some(()).unwrap();
    }
}
