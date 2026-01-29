use std::fmt::Debug;
use std::hash::Hash;

pub enum Builder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    TcpClient(net::tcp::ClientBuilder<T>),
    TcpServer(net::tcp::ServerBuilder<T>),
    RtuClient(net::rtu::ClientBuilder<T>),
    RtuServer(net::rtu::ServerBuilder<T>),
}
