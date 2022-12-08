use std::str::FromStr;

use light_rpc::bridge::LightBridge;
use reqwest::Url;
use simplelog::*;

const RPC_ADDR: &str = "http://5.62.126.197:9000/";
const WS_ADDR: &str = "ws://127.0.0.1:8900";

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    let light_bridge = LightBridge::new(
        Url::from_str(RPC_ADDR).unwrap(),
        WS_ADDR,
    )
    .await
    .unwrap();

    light_bridge.start_server("127.0.0.1:8890").await
}
