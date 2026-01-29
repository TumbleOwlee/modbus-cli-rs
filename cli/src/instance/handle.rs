use std::fmt::Debug;
use std::hash::Hash;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub enum Client {
    Tcp(net::tcp::Client),
    Rtu(net::rtu::Client),
}

pub enum Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
{
    Tcp(net::tcp::Server<T, L>),
    Rtu(net::rtu::Server<T, L>),
}

pub struct ClientHandle {
    pub handle: JoinHandle<Result<(), net::Error>>,
    pub sender: Sender<net::Command>,
}

pub struct ServerHandle {
    pub handle: JoinHandle<Result<(), net::Error>>,
}

pub enum Handle {
    Server(ServerHandle),
    Client(ClientHandle),
}
