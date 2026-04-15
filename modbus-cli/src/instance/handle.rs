use std::fmt::Debug;
use std::hash::Hash;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub enum Client {
    Tcp(modbus_net::tcp::Client),
    Rtu(modbus_net::rtu::Client),
}

pub enum Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
{
    Tcp(modbus_net::tcp::Server<T, L>),
    Rtu(modbus_net::rtu::Server<T, L>),
}

pub struct ClientHandle {
    pub handle: JoinHandle<Result<(), modbus_net::Error>>,
    pub sender: Sender<modbus_net::Command>,
}

pub struct ServerHandle {
    pub handle: JoinHandle<Result<(), modbus_net::Error>>,
}

pub enum Handle {
    Server(ServerHandle),
    Client(ClientHandle),
}
