pub mod client;
pub mod server;

#[derive(Clone, Debug, Default)]
pub struct TcpConfig {
    pub port: u16,
    pub ip: String,
    pub interval_ms: u64,
}
