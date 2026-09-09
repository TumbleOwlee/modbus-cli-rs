#[derive(Debug)]
pub struct TcpPortGuard {
    listener: std::net::TcpListener,
    port: u16,
}

pub fn reserve_tcp_port() -> TcpPortGuard {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("binding 127.0.0.1:0 cannot fail on a loopback interface");
    let port = listener
        .local_addr()
        .expect("a bound TcpListener always has a local address")
        .port();
    TcpPortGuard { listener, port }
}

impl TcpPortGuard {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn into_listener(self) -> std::net::TcpListener {
        self.listener
    }

    /// NF-R-057's sole sanctioned time-of-check/time-of-use window: drops
    /// the binding and returns the port number, for a server that can only
    /// bind by number.
    pub fn release(self) -> u16 {
        self.port
    }
}

#[derive(Debug)]
pub struct UdpPortGuard {
    socket: std::net::UdpSocket,
    port: u16,
}

pub fn reserve_udp_port() -> UdpPortGuard {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("binding 127.0.0.1:0 cannot fail on a loopback interface");
    let port = socket
        .local_addr()
        .expect("a bound UdpSocket always has a local address")
        .port();
    UdpPortGuard { socket, port }
}

impl UdpPortGuard {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn into_socket(self) -> std::net::UdpSocket {
        self.socket
    }

    /// NF-R-057's sole sanctioned time-of-check/time-of-use window: drops
    /// the binding and returns the port number, for a server that can only
    /// bind by number.
    pub fn release(self) -> u16 {
        self.port
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NF-R-043 — a held TCP guard keeps the port bound, so a second bind on
    /// the same port fails with `AddrInUse`.
    #[test]
    fn ut_reserve_tcp_port_holds_binding() {
        let guard = reserve_tcp_port();
        let err = std::net::TcpListener::bind(("127.0.0.1", guard.port())).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }

    /// NF-R-066 — `release()` drops the binding and hands back the number,
    /// which is then free to bind again. This rebind is a real TOCTOU window: any concurrent
    /// `bind(":0")`, in this process (a sibling test on another thread) or another, can steal
    /// the number first and make this assertion fail spuriously.
    #[test]
    fn ut_tcp_release_frees_port() {
        let guard = reserve_tcp_port();
        let port = guard.port();
        assert_eq!(guard.release(), port);
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    /// NF-R-064, NF-R-065 — `port()` reports the bound port, and `into_listener()` hands over
    /// that same bound socket, not a fresh one.
    #[test]
    fn ut_tcp_into_listener_keeps_binding() {
        let guard = reserve_tcp_port();
        let port = guard.port();
        let listener = guard.into_listener();
        assert_eq!(listener.local_addr().unwrap().port(), port);
    }

    /// NF-R-043 — a held UDP guard keeps the port bound, so a second bind on
    /// the same port fails with `AddrInUse`.
    #[test]
    fn ut_reserve_udp_port_holds_binding() {
        let guard = reserve_udp_port();
        let err = std::net::UdpSocket::bind(("127.0.0.1", guard.port())).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }

    /// NF-R-066 — `release()` drops the binding and hands back the number,
    /// which is then free to bind again. This rebind is a real TOCTOU window: any concurrent
    /// `bind(":0")`, in this process (a sibling test on another thread) or another, can steal
    /// the number first and make this assertion fail spuriously.
    #[test]
    fn ut_udp_release_frees_port() {
        let guard = reserve_udp_port();
        let port = guard.port();
        assert_eq!(guard.release(), port);
        std::net::UdpSocket::bind(("127.0.0.1", port)).unwrap();
    }

    /// NF-R-064, NF-R-065 — `port()` reports the bound port, and `into_socket()` hands over that
    /// same bound socket, not a fresh one.
    #[test]
    fn ut_udp_into_socket_keeps_binding() {
        let guard = reserve_udp_port();
        let port = guard.port();
        let socket = guard.into_socket();
        assert_eq!(socket.local_addr().unwrap().port(), port);
    }
}
