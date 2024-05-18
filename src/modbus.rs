use crate::memory::{Memory, Range};

use std::{
    collections::HashMap,
    future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_modbus::{
    prelude::*,
    server::tcp::{accept_tcp_connection, Server as TcpServer},
};

pub struct Server<const SLICE_SIZE: usize> {
    memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
}

impl<const SLICE_SIZE: usize> tokio_modbus::server::Service for Server<SLICE_SIZE> {
    type Request = Request<'static>;
    type Future = future::Ready<Result<Response, Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        match req {
            Request::ReadInputRegisters(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(&Range::new(addr, addr + cnt))
                    .map_err(|_| Exception::IllegalDataAddress)
                    .map(|v| Response::ReadInputRegisters(v.into_iter().copied().collect())),
            ),
            Request::ReadHoldingRegisters(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(&Range::new(addr, addr + cnt))
                    .map_err(|_| Exception::IllegalDataAddress)
                    .map(|v| Response::ReadHoldingRegisters(v.into_iter().copied().collect())),
            ),
            Request::WriteMultipleRegisters(addr, values) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .write(Range::new(addr, addr + (values.len() as u16)), &values)
                    .map_err(|_| Exception::IllegalDataAddress)
                    .map(|_| Response::WriteMultipleRegisters(addr, values.len() as u16)),
            ),
            Request::WriteSingleRegister(addr, value) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .write(Range::new(addr, addr + 1), &[value])
                    .map_err(|_| Exception::IllegalDataAddress)
                    .map(|_| Response::WriteSingleRegister(addr, value)),
            ),
            _ => future::ready(Err(Exception::IllegalFunction)),
        }
    }
}

impl<const SLICE_SIZE: usize> Server<SLICE_SIZE> {
    pub fn new(memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>) -> Self {
        Self { memory }
    }
}

//async fn client_context(socket_addr: SocketAddr) {
//    tokio::join!(
//        async {
//            // Give the server some time for starting up
//            tokio::time::sleep(Duration::from_secs(1)).await;
//
//            println!("CLIENT: Connecting client...");
//            let mut ctx = tcp::connect(socket_addr).await.unwrap();
//
//            println!("CLIENT: Reading 2 input registers...");
//            let response = ctx.read_input_registers(0x00, 2).await.unwrap();
//            println!("CLIENT: The result is '{response:?}'");
//            assert_eq!(response.unwrap(), vec![1234, 5678]);
//
//            println!("CLIENT: Writing 2 holding registers...");
//            ctx.write_multiple_registers(0x01, &[7777, 8888])
//                .await
//                .unwrap()
//                .unwrap();
//
//            // Read back a block including the two registers we wrote.
//            println!("CLIENT: Reading 4 holding registers...");
//            let response = ctx.read_holding_registers(0x00, 4).await.unwrap();
//            println!("CLIENT: The result is '{response:?}'");
//            assert_eq!(response.unwrap(), vec![10, 7777, 8888, 40]);
//
//            // Now we try to read with an invalid register address.
//            // This should return a Modbus exception response with the code
//            // IllegalDataAddress.
//            println!("CLIENT: Reading nonexistent holding register address... (should return IllegalDataAddress)");
//            let response = ctx.read_holding_registers(0x100, 1).await.unwrap();
//            println!("CLIENT: The result is '{response:?}'");
//            assert!(matches!(response, Err(Exception::IllegalDataAddress)));
//
//            println!("CLIENT: Done.")
//        },
//        tokio::time::sleep(Duration::from_secs(5))
//    );
//}
