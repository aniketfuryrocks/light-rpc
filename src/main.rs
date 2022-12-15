use std::str::FromStr;

use light_rpc::bridge::LightBridge;
use light_rpc::{DEFAULT_RPC_ADDR, DEFAULT_WS_ADDR};
use reqwest::Url;
use simplelog::*;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    let light_bridge =
        LightBridge::new(Url::from_str(DEFAULT_RPC_ADDR).unwrap(), DEFAULT_WS_ADDR).await?;

    let services = light_bridge.start_services("127.0.0.1:8890");
    let services = futures::future::join_all(services);

    let ctrl_c_signal = tokio::signal::ctrl_c();

    tokio::select! {
        services = services => {
            for res in services {
                res??;
            }
            anyhow::bail!("Some services exited unexpectedly")
        }
        _ = ctrl_c_signal => {
            Ok(())
        }
    }
}
