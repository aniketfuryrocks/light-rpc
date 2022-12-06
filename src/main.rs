use std::str::FromStr;

use light_rpc::bridge::LightBridge;
use reqwest::Url;

const RPC_ADDR: &str = "http://127.0.0.1:8899";
const TPU_ADDR: &str = "127.0.0.1:1027";
const WS_ADDR: &str = "ws://127.0.0.1:8900";
const CONNECTION_POOL_SIZE: usize = 1024;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let light_bridge = LightBridge::new(
        Url::from_str(RPC_ADDR).unwrap(),
        TPU_ADDR.parse().unwrap(),
        WS_ADDR,
        CONNECTION_POOL_SIZE,
    )
    .await
    .unwrap();

    light_bridge.start_server("127.0.0.1:8890").await
}
