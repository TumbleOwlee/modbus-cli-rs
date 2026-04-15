use modbus_mem::Memory;

use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ClientConfig<T, Config>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub id: T,
    pub config: Arc<RwLock<Config>>,
    pub operations: Arc<RwLock<Vec<modbus_net::Operation>>>,
    pub memory: Arc<RwLock<Memory<modbus_net::Key<T>>>>,
}

#[derive(Clone)]
pub struct ServerConfig<T, Config>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub id: T,
    pub config: Arc<RwLock<Config>>,
    pub memory: Arc<RwLock<Memory<modbus_net::Key<T>>>>,
}
