use crate::memory::{Memory, Range};

use std::{
    future,
    sync::{Arc, Mutex},
};
use tokio_modbus::prelude::{Exception, Request, Response};

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
