use crate::mem::data::DataType;
use crate::mem::memory::{Memory, Range};
use crate::mem::register::{AccessType, Definition};
use crate::msg::LogMsg;
use crate::tcp::TcpConfig;
use crate::util::{str, Expect};
use crate::{AppConfig, Command, Status};

use itertools::Itertools;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_modbus::prelude::{Client as ModbusClient, Reader, SlaveContext, SlaveId, Writer};
use tokio_modbus::{FunctionCode, Slave};

pub struct Client {
    tcp_config: TcpConfig,
    memory: Arc<Mutex<Memory>>,
    operations: Vec<(SlaveId, FunctionCode, Range<usize>)>,
    status_sender: Sender<Status>,
    cmd_receiver: Receiver<Command>,
    log_sender: Sender<LogMsg>,
}

impl Client {
    pub fn new(
        app_config: Arc<Mutex<AppConfig>>,
        tcp_config: TcpConfig,
        memory: Arc<Mutex<Memory>>,
        status_sender: Sender<Status>,
        cmd_receiver: Receiver<Command>,
        log_sender: Sender<LogMsg>,
    ) -> Self {
        let operations = Self::init(app_config);
        Self {
            tcp_config,
            memory,
            operations,
            status_sender,
            cmd_receiver,
            log_sender,
        }
    }

    fn init(app_config: Arc<Mutex<AppConfig>>) -> Vec<(SlaveId, FunctionCode, Range<usize>)> {
        let config = app_config.lock().unwrap();
        let mut sorted_defs = config
            .definitions
            .iter()
            .sorted_by(|a, b| {
                a.1.get_slave_id()
                    .unwrap_or(0)
                    .cmp(&b.1.get_slave_id().unwrap_or(0))
                    .then(
                        a.1.read_code()
                            .cmp(&b.1.read_code())
                            .then(a.1.get_address().cmp(&b.1.get_address())),
                    )
            })
            .collect::<Vec<_>>();

        let marker = (
            str!(""),
            Definition::new(
                None,
                0,
                0,
                DataType::default(),
                0,
                AccessType::ReadWrite,
                None,
            ),
        );
        sorted_defs.push((&marker.0, &marker.1));

        let is_allowed = |slave: SlaveId, fc: u8, addr: u16, end: usize| {
            for mem in config.contiguous_memory.iter() {
                if mem.slave_id.unwrap_or(0) == slave
                    && mem.read_code == fc
                    && addr as usize >= mem.range.start()
                    && addr as usize <= mem.range.end()
                    && end >= mem.range.start()
                    && end <= mem.range.end()
                {
                    return true;
                }
            }
            false
        };

        let mut fc: i16 = -1;
        let mut slave: i16 = -1;
        let mut operations = Vec::new();
        let mut range: Option<Range<u16>> = None;
        for (name, def) in sorted_defs.into_iter() {
            if let Some(ref mut range) = range {
                let (def_slave, def_addr, def_fc, def_range) = (
                    def.get_slave_id().unwrap_or(0),
                    def.get_address(),
                    def.read_code(),
                    def.get_range(),
                );
                if (fc != -1
                    && slave != -1
                    && ((fc != (def_fc as i16)) || (slave != (def_slave as i16))))
                    || ((range.length() + def_addr as usize + def_range.length())
                        > (range.start() + 127))
                    || (def_addr >= range.end() as u16
                        && !is_allowed(slave as SlaveId, fc as u8, def_addr, range.end()))
                {
                    let fc = FunctionCode::new(fc as u8);
                    match fc {
                        FunctionCode::ReadCoils
                        | FunctionCode::ReadDiscreteInputs
                        | FunctionCode::ReadInputRegisters
                        | FunctionCode::ReadHoldingRegisters => {
                            let len = range.length();
                            if len > 0 {
                                let mut addr = range.start();
                                loop {
                                    operations.push((
                                        slave as SlaveId,
                                        fc,
                                        Range::new(addr, std::cmp::min(addr + 127, range.end())),
                                    ));
                                    addr = std::cmp::min(addr + 127, range.end());
                                    if addr == range.end() {
                                        break;
                                    }
                                }
                            }
                        }
                        _ => panic!("Invalid read function code for register {}", name),
                    };
                    Range::new(def_addr, def_addr).clone_into(range);
                }
            }
            fc = def.read_code() as i16;
            slave = def.get_slave_id().unwrap_or(0) as i16;
            range = match range {
                None => Some(def.get_range()),
                Some(v) => Some(Range::new(v.start() as u16, def.get_range().end() as u16)),
            };
        }
        operations
    }

    pub async fn run(&mut self, delay_after_connect: u64, interval_ms: u64, timeout_ms: u64) {
        let addr: SocketAddr = format!("{}:{}", self.tcp_config.ip, self.tcp_config.port)
            .parse()
            .panic(|e| format!("Failed to create SocketAddr ({e})"));
        let mut connection = if let Ok(r) = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            tokio_modbus::client::tcp::connect(addr),
        )
        .await
        {
            r.ok()
        } else {
            None
        };
        if connection.is_some() {
            let _ = self
                .status_sender
                .send(Status::String(str!("Modbus TCP connected.")))
                .await;
            let _ = self
                .log_sender
                .send(LogMsg::ok(&format!(
                    "Modbus TCP connected to {}:{}",
                    self.tcp_config.ip, self.tcp_config.port
                )))
                .await;
        } else {
            let _ = self
                .status_sender
                .send(Status::String(str!("Modbus TCP disconnected.")))
                .await;
            let _ = self
                .log_sender
                .send(LogMsg::err(&format!(
                    "Modbus TCP failed to connect to {}:{}",
                    self.tcp_config.ip, self.tcp_config.port
                )))
                .await;
        };

        tokio::time::sleep(tokio::time::Duration::from_millis(delay_after_connect)).await;

        let mut time_last_read = SystemTime::now()
            .checked_sub(Duration::from_millis(interval_ms + 1))
            .unwrap();
        let mut op_idx = 0;
        let mut retries = 0;
        loop {
            if let Some(ref mut context) = connection {
                let mut reconnect = false;
                let mut disconnect = false;

                // Perform next read of registers
                let now = SystemTime::now();
                let res = now.duration_since(time_last_read);
                if res.is_ok_and(|d| d.as_millis() > interval_ms as u128) {
                    time_last_read = now;
                    let (slave, fc, op) = self.operations.get(op_idx).unwrap();
                    let modbus_result = match fc {
                        FunctionCode::ReadCoils => {
                            context.set_slave(Slave(*slave));
                            tokio::time::timeout(
                                Duration::from_millis(timeout_ms),
                                context
                                    .read_coils(op.start() as u16, (op.end() - op.start()) as u16),
                            )
                            .await
                            .map(|r| {
                                r.map(|v| {
                                    v.map(|b| {
                                        b.into_iter().map(|e| if e { 1 } else { 0 }).collect()
                                    })
                                })
                            })
                        }
                        FunctionCode::ReadDiscreteInputs => {
                            context.set_slave(Slave(*slave));
                            tokio::time::timeout(
                                Duration::from_millis(timeout_ms),
                                context.read_discrete_inputs(
                                    op.start() as u16,
                                    (op.end() - op.start()) as u16,
                                ),
                            )
                            .await
                            .map(|r| {
                                r.map(|v| {
                                    v.map(|b| {
                                        b.into_iter().map(|e| if e { 1 } else { 0 }).collect()
                                    })
                                })
                            })
                        }
                        FunctionCode::ReadInputRegisters => {
                            context.set_slave(Slave(*slave));
                            tokio::time::timeout(
                                Duration::from_millis(timeout_ms),
                                context.read_input_registers(
                                    op.start() as u16,
                                    (op.end() - op.start()) as u16,
                                ),
                            )
                            .await
                        }
                        FunctionCode::ReadHoldingRegisters => {
                            context.set_slave(Slave(*slave));
                            tokio::time::timeout(
                                Duration::from_millis(timeout_ms),
                                context.read_holding_registers(
                                    op.start() as u16,
                                    (op.end() - op.start()) as u16,
                                ),
                            )
                            .await
                        }
                        _ => panic!("Invalid function code in operation."),
                    };
                    if let Ok(Ok(Ok(vec))) = modbus_result {
                        let _ = self.log_sender
                            .send(LogMsg::info(&format!(
                                "Read address space [ {start:#06X} ({start}), {end:#06X} ({end}) ) successful.",
                                start = op.start(),
                                end = op.start()
                            )))
                            .await;
                        let mut memory = self.memory.lock().unwrap();
                        memory
                            .write(*slave, Range::new(op.start(), op.start() + vec.len()), &vec)
                            .panic(|e| format!("Failed to write to memory ({})", e));
                        drop(memory);
                        op_idx = if op_idx + 1 == self.operations.len() {
                            0
                        } else {
                            op_idx + 1
                        };
                        retries = 0;
                    } else {
                        retries += 1;
                        if retries > 3 {
                            op_idx = if op_idx + 1 == self.operations.len() {
                                0
                            } else {
                                op_idx + 1
                            };
                            retries = 0;
                        }

                        let _ = self.log_sender
                            .send(LogMsg::err(&format!(
                                "Read address space [ {start:#06X} ({start}), {end:#06X} ({end}) ) failed.",
                                start = op.start(),
                                end = op.start()
                            )))
                            .await;
                        let _ = self
                            .status_sender
                            .send(Status::String(str!("Modbus TCP disconnected.")))
                            .await;
                        reconnect = true;
                    }
                }

                // Execute next command if available
                if let Ok(cmd) = self.cmd_receiver.try_recv() {
                    match cmd {
                        Command::Disconnect => {
                            let _ = self
                                .status_sender
                                .send(Status::String(str!("Modbus TCP disconnected.")))
                                .await;
                            let _ = self
                                .log_sender
                                .send(LogMsg::ok(&format!(
                                    "Modbus TCP disconnected from {}:{}",
                                    self.tcp_config.ip, self.tcp_config.port
                                )))
                                .await;
                            disconnect = true;
                        }
                        Command::Connect => {
                            reconnect = true;
                        }
                        Command::WriteSingleCoil((slave, addr, coil)) => {
                            context.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                context.write_single_coil(addr, coil),
                            )
                            .await
                            {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::err(&format!(
                                        "Failed to write address {addr} with values {coil:?} [{e}]."
                                    )))
                                    .await;
                                reconnect = true;
                            } else {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::ok(&format!(
                                        "Successfully written address {addr} with values {coil:?}."
                                    )))
                                    .await;
                            }
                        }
                        Command::WriteMultipleCoils((slave, addr, coils)) => {
                            context.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                context.write_multiple_coils(addr, &coils),
                            )
                            .await
                            {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::err(&format!(
                                        "Failed to write address {addr} with values {coils:?} [{e}]."
                                    )))
                                    .await;
                                reconnect = true;
                            } else {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::ok(&format!(
                                        "Successfully written address {addr} with values {coils:?}."
                                    )))
                                    .await;
                            }
                        }
                        Command::WriteSingleRegister((slave, addr, value)) => {
                            context.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                context.write_single_register(addr, value),
                            )
                            .await
                            {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::err(&format!(
                                        "Failed to write address {addr} with values {value:?} [{e}]."
                                    )))
                                    .await;
                                reconnect = true;
                            } else {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::ok(&format!(
                                        "Successfully written address {addr} with values {value:?}."
                                    )))
                                    .await;
                            }
                        }
                        Command::WriteMultipleRegisters((slave, addr, vec)) => {
                            context.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                context.write_multiple_registers(addr, &vec),
                            )
                            .await
                            {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::err(&format!(
                                        "Failed to write address {addr} with values {vec:?} [{e}]."
                                    )))
                                    .await;
                                reconnect = true;
                            } else {
                                let _ = self
                                    .log_sender
                                    .send(LogMsg::ok(&format!(
                                        "Successfully written address {addr} with values {vec:?}."
                                    )))
                                    .await;
                            }
                        }
                    }
                }

                if disconnect {
                    if let Some(mut conn) = connection.take() {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms),
                            conn.disconnect(),
                        )
                        .await;
                    }
                }

                // Reset connection on error
                if reconnect {
                    connection = if let Ok(r) = tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        tokio_modbus::client::tcp::connect(addr),
                    )
                    .await
                    {
                        r.ok()
                    } else {
                        None
                    };
                    if connection.is_some() {
                        let _ = self
                            .status_sender
                            .send(Status::String(str!("Modbus TCP connected.")))
                            .await;
                        let _ = self
                            .log_sender
                            .send(LogMsg::ok(&format!(
                                "Modbus TCP reconnected successfully to {}:{}",
                                self.tcp_config.ip, self.tcp_config.port
                            )))
                            .await;
                    } else {
                        let _ = self
                            .log_sender
                            .send(LogMsg::err(&format!(
                                "Modbus TCP failed to reconnect to {}:{}",
                                self.tcp_config.ip, self.tcp_config.port
                            )))
                            .await;
                    }
                }
            } else if let Ok(cmd) = self.cmd_receiver.try_recv() {
                match cmd {
                    Command::Connect => {
                        connection = if let Ok(r) = tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms),
                            tokio_modbus::client::tcp::connect(addr),
                        )
                        .await
                        {
                            r.ok()
                        } else {
                            None
                        };
                        if connection.is_some() {
                            let _ = self
                                .status_sender
                                .send(Status::String(str!("Modbus TCP connected.")))
                                .await;
                            let _ = self
                                .log_sender
                                .send(LogMsg::ok(&format!(
                                    "Modbus TCP connected successfully to {}:{}",
                                    self.tcp_config.ip, self.tcp_config.port
                                )))
                                .await;
                        } else {
                            let _ = self
                                .log_sender
                                .send(LogMsg::err(&format!(
                                    "Modbus TCP failed to connect to {}:{}",
                                    self.tcp_config.ip, self.tcp_config.port
                                )))
                                .await;
                        }
                        op_idx = 0;
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}
