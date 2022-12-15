pub mod bridge;
pub mod configs;
pub mod encoding;
pub mod rpc;
pub mod workers;

pub type WireTransaction = Vec<u8>;

pub const DEFAULT_RPC_ADDR: &str = "http://127.0.0.1:8899";
pub const DEFAULT_WS_ADDR: &str = "ws://127.0.0.1:8900";
pub const DEFAULT_TX_MAX_RETRIES: u16 = 2;
pub const DEFAULT_TX_RETRY_BATCH_SIZE: usize = 20;
