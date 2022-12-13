use std::ops::{Deref, DerefMut};

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::RpcRequest;
use solana_client::rpc_response::Response as RpcResponse;

pub struct LightClient(pub RpcClient);

impl Deref for LightClient {
    type Target = RpcClient;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LightClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl LightClient {
    pub async fn confirm_transaction(&self, signature: String) -> RpcResponse<bool> {
        self.send(
            RpcRequest::Custom {
                method: "confirmTransaction",
            },
            serde_json::json!([signature]),
        )
        .await
        .unwrap()
    }
}
