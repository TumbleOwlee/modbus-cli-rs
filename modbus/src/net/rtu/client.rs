use crate::mem::layout::Format;
use crate::mem::memory::Memory;
use crate::mem::range::Range;
use crate::mem::register::{AccessType, Definition};
use crate::msg::LogMsg;
use crate::rtu::Config;
use crate::util::{str, Expect};
use crate::{Command, Status};

use itertools::Itertools;
use tokio_modbus::client::Context;
use std::time::{Duration, Instant};
use tokio_modbus::prelude::SlaveId;
use tokio_modbus::prelude::{rtu, Client as ModbusClient, Reader, Slave, SlaveContext, Writer};
use tokio_modbus::FunctionCode;
use tokio_serial::SerialStream;
use tokio::time::Duration;
use tokio::time::sleep;
use anyhow::{anyhow, Error};

use crate::mem::{Request as MemRequest, Response as MemResponse};
use crate::ui::{Request as UiRequest, Response as UiResponse};
use crate::sync::channel::{DuplexChannel, DuplexChannelPair};
use crate::net::Config as NetConfig;


pub struct Operation {
    pub slave_id: SlaveId,
    pub fn_code: FunctionCode,
    pub range: Range,
}

pub enum Request {
    Shutdown,
    Connect,
    Disconnect,
    WriteSingleCoil((SlaveId, u16, bool)),
    WriteMultipleCoils((SlaveId, u16, Vec<bool>)),
    WriteSingleRegister((SlaveId, u16, u16)),
    WriteMultipleRegisters((SlaveId, u16, Vec<u16>)),
}

pub enum Response {
    Confirm,
}

pub struct Client {
    config: Config,
    net_config: NetConfig,
    operations: Vec<Operation>,
    channels: Vec<DuplexChannel<Response, Request>>,
    memory: Option<DuplexChannel<MemRequest<SlaveId>, MemResponse>>,
    ui: Option<DuplexChannel<UiRequest, UiResponse>>,
}

impl Client {
    pub fn new(config: Config, net_config: NetConfig, operations: Vec<Operation>) -> Self {
        Self {
            config,
            net_config,
            operations,
            channels: vec![],
            memory: None,
            ui: None,
        }
    }

    pub fn get_channel(&mut self, size: usize) -> DuplexChannel<Request, Response> {
        let (c1, c2) = DuplexChannelPair::new(size).split();
        self.channels.push(c2);
        c1
    }

    pub fn attach_channel(&mut self, channel: DuplexChannel<Response, Request>) -> () {
        self.channels.push(channel);
    }

    pub fn attach_ui(&mut self, channel: DuplexChannel<UiRequest, UiResponse>) -> () {
        self.ui = Some(channel);
    }

    pub fn attach_memory(
        &mut self,
        channel: DuplexChannel<MemRequest<SlaveId>, MemResponse>,
    ) -> () {
        self.memory = Some(channel);
    }

//    fn init(app_config: Arc<Mutex<AppConfig>>) -> Vec<(SlaveId, FunctionCode, Range<usize>)> {
//        let config = app_config.lock().unwrap();
//        let mut sorted_defs = config
//            .definitions
//            .iter()
//            .filter(|d| !d.1.is_virtual())
//            .sorted_by(|a, b| {
//                a.1.get_slave_id()
//                    .unwrap_or(0)
//                    .cmp(&b.1.get_slave_id().unwrap_or(0))
//                    .then(
//                        a.1.read_code()
//                            .cmp(&b.1.read_code())
//                            .then(a.1.get_address().cmp(&b.1.get_address())),
//                    )
//            })
//            .collect::<Vec<_>>();
//
//        let marker = (
//            str!(""),
//            Definition::new(
//                None,
//                None,
//                0,
//                0,
//                DataType::default(),
//                0,
//                AccessType::ReadOnly,
//                None,
//                None,
//                None,
//                None,
//            ),
//        );
//        sorted_defs.push((&marker.0, &marker.1));
//
//        let is_allowed = |fc: u8, addr: u16, end: usize| {
//            for mem in config.contiguous_memory.iter() {
//                if mem.read_code == fc
//                    && addr as usize >= mem.range.start()
//                    && addr as usize <= mem.range.end()
//                    && end >= mem.range.start()
//                    && end <= mem.range.end()
//                {
//                    return true;
//                }
//            }
//            false
//        };
//
//        let mut fc: i16 = -1;
//        let mut slave: i16 = -1;
//        let mut operations = Vec::new();
//        let mut range: Option<Range<u16>> = None;
//        for (name, def) in sorted_defs.into_iter() {
//            if let Some(ref mut range) = range {
//                let (def_slave, def_addr, def_fc, def_range) = (
//                    def.get_slave_id(),
//                    def.get_address(),
//                    def.read_code(),
//                    def.get_range(),
//                );
//                if (fc != -1 && fc != (def_fc as i16))
//                    || (slave != (def_slave.unwrap_or(0) as i16))
//                    || ((range.length() + def_addr as usize + def_range.length())
//                        > (range.start() + 127))
//                    || (def_addr >= range.end() as u16
//                        && !is_allowed(fc as u8, def_addr, range.end()))
//                {
//                    let fc = FunctionCode::new(fc as u8);
//                    match fc {
//                        FunctionCode::ReadCoils
//                        | FunctionCode::ReadDiscreteInputs
//                        | FunctionCode::ReadInputRegisters
//                        | FunctionCode::ReadHoldingRegisters => {
//                            let len = range.length();
//                            if len > 0 {
//                                let mut addr = range.start();
//                                loop {
//                                    operations.push((
//                                        slave as SlaveId,
//                                        fc,
//                                        Range::new(addr, std::cmp::min(addr + 127, range.end())),
//                                    ));
//                                    addr = std::cmp::min(addr + 127, range.end());
//                                    if addr == range.end() {
//                                        break;
//                                    }
//                                }
//                            }
//                        }
//                        _ => panic!("Invalid read function code for register {}", name),
//                    };
//                    Range::new(def_addr, def_addr).clone_into(range);
//                }
//            }
//            fc = def.read_code() as i16;
//            slave = def.get_slave_id().unwrap_or(0) as i16;
//            range = match range {
//                None => Some(def.get_range()),
//                Some(v) => Some(Range::new(v.start() as u16, def.get_range().end() as u16)),
//            };
//        }
//        operations
//    }

    async fn read(&mut self, ctx: &mut Context, op: &Operation) -> Result<Vec<u16>, anyhow::Error> {
        let result = match op.fn_code {
            FunctionCode::ReadCoils => {
                ctx.set_slave(Slave(op.slave_id));
                tokio::time::timeout(
                    Duration::from_millis(self.net_config.timeout_ms as u64),
                    ctx
                        .read_coils(op.range.start as u16, (op.range.end - op.range.start) as u16),
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
                ctx.set_slave(Slave(op.slave_id));
                tokio::time::timeout(
                    Duration::from_millis(self.net_config.timeout_ms as u64),
                    ctx.read_discrete_inputs(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
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
                ctx.set_slave(Slave(op.slave_id));
                tokio::time::timeout(
                    Duration::from_millis(self.net_config.timeout_ms as u64),
                    ctx.read_input_registers(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await
            }
            FunctionCode::ReadHoldingRegisters => {
                ctx.set_slave(Slave(op.slave_id));
                tokio::time::timeout(
                    Duration::from_millis(self.net_config.timeout_ms as u64),
                    ctx.read_holding_registers(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await
            }
            _ => panic!("Invalid function code in operation."),
        };
        match result {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("{}", e)),
            Ok(Err(e)) => Err(anyhow!("{}", e)),
            Err(e) => Err(anyhow!("{}", e)),
        }
    }

    async fn create_context(&mut self) -> Option<Context> {
        let builder = tokio_serial::new(self.config.path.clone(), self.config.baud_rate);
        if let Ok(ctx) = SerialStream::open(&builder).map(|s| rtu::attach_slave(s, Slave(self.config.slave))) {
            self.ui.iter().map(async |channel| {
                channel.send(UiRequest::Status(str!("Modbus RTU connected."))).await;
                channel.send(UiRequest::LogInfo(format!(
                    "Modbus RTU successfully connected to {} with baud rate {}",
                    self.config.path, self.config.baud_rate
                ))).await;
            });
            Some(ctx)
        } else {
            self.ui.iter().map(async |channel| {
                channel.send(UiRequest::Status(str!("Modbus RTU disconnected."))).await;
                channel.send(UiRequest::LogError(format!(
                    "Modbus RTU failed to connect to {} with baud rate {}",
                    self.config.path, self.config.baud_rate
                ))).await;
            });
            None
        }
    }

    pub fn interval_elapsed(&self, since: &mut Option<Instant>) -> bool {
        let now = Instant::now();
        match since {
            Some(time) => {
                let duration = now.duration_since(*time);
                if duration.as_millis() > self.net_config.interval_ms as u128 {
                    *since = Some(now);
                    true
                } else {
                    false
                }
            }
            None => {
                    *since = Some(now);
                true
            }
        }
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        let delay = self.net_config.delay_after_connect_ms as u64;
        let mut time: Option<Instant> = None;

        let mut context = self.create_context().await;
        sleep(Duration::from_millis(delay)).await;

        let mut index = 0;
        let mut retries = 0;
        loop {
            if let Some(ref mut ctx) = context {
                let mut reconnect = false;
                let mut disconnect = false;

                // Perform next read of registers
                if self.interval_elapsed(&mut time) {
                    let op = self.operations.get(index).unwrap();
                    match self.read(ctx, op).await {
                        Ok(vec) if self.memory.is_some() => {
                            self.ui.iter().map(async |channel| {
                                channel.send(UiRequest::LogInfo(format!(
                                    "Read address space [ {start:#06X} ({start}), {end:#06X} ({end}) ) successful.",
                                    start = op.range.start,
                                    end = op.range.end
                                ))).await;
                            });
                            let memory = self.memory.as_ref().unwrap();
                            match memory.send(MemRequest::Write((op.slave_id, op.range.clone(), vec))).await {
                                Ok(()) => {}
                                Err(e) => {
                                    self.ui.iter().map(async |channel| {
                                        channel.send(UiRequest::LogError(format!(
                                            "Failed to request memory write for [ {start:#06X} ({start}), {end:#06X} ({end}) ).",
                                            start = op.range.start,
                                            end = op.range.end
                                        ))).await;
                                    });
                                }
                            }
                            index = (index + 1) % self.operations.len();
                            retries = 0;
                        }
                        _ => {
                            retries += 1;
                            if retries > 3 {
                                index = (index + 1) % self.operations.len();
                                retries = 0;
                            }

                            self.ui.iter().map(async |channel| {
                                channel.send(UiRequest::LogError(format!(
                                    "Read address space [ {start:#06X} ({start}), {end:#06X} ({end}) ) failed.",
                                    start = op.range.start,
                                    end = op.range.end
                                ))).await;
                            });
                            reconnect = true;
                        }
                    }
                }

                // Execute next command if available
                if let Ok(cmd) = self.cmd_receiver.try_recv() {
                    match cmd {
                        Command::Disconnect => {
                            let _ = self
                                .status_sender
                                .send(Status::String(str!("Modbus RTU disconnected.")))
                                .await;
                            let _ = self
                                .log_sender
                                .send(LogMsg::ok(&format!(
                                    "Modbus RTU disconnected from {} with baud rate {}",
                                    self.rtu_config.path, self.rtu_config.baud_rate
                                )))
                                .await;
                            disconnect = true;
                        }
                        Command::Connect => {
                            reconnect = true;
                        }
                        Command::WriteSingleCoil((slave, addr, coil)) => {
                            ctx.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                ctx.write_single_coil(addr, coil),
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
                            ctx.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                ctx.write_multiple_coils(addr, &coils),
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
                            ctx.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                ctx.write_single_register(addr, value),
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
                            ctx.set_slave(Slave(slave));
                            if let Err(e) = tokio::time::timeout(
                                std::time::Duration::from_millis(timeout_ms),
                                ctx.write_multiple_registers(addr, &vec),
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
                    let builder =
                        tokio_serial::new(self.rtu_config.path.clone(), self.rtu_config.baud_rate);
                    let port = SerialStream::open(&builder)
                        .panic(|e| format!("Failed to open SerialStream ({e})"));
                    let slave = Slave(self.rtu_config.slave);
                    connection = Some(rtu::attach_slave(port, slave));
                    if connection.is_some() {
                        let _ = self
                            .status_sender
                            .send(Status::String(str!("Modbus TCP connected.")))
                            .await;
                        let _ = self
                            .log_sender
                            .send(LogMsg::ok(&format!(
                                "Modbus RTU reconnected successfully to {} with baud rate {}",
                                self.rtu_config.path, self.rtu_config.baud_rate
                            )))
                            .await;
                    } else {
                        let _ = self
                            .log_sendSlaveId>
                            .send(LogMsg::err(&format!(
                                "Modbus RTU failed to reconnect to {} with baud rate {}",
                                self.rtu_config.path, self.rtu_config.baud_rSlaveId>e
                            )))
                            .await;
                    }
                }
            } else if let Ok(Command::Connect) = self.cmd_receiver.try_recv() {
                let builder =
                    tokio_serial::new(self.rtu_config.path.clone(), self.rtu_config.baud_rate);
                let port = SerialStream::open(&builder)
                    .panic(|e| format!("Failed to open SerialStream ({e})"));
                let slave = Slave(self.rtu_config.slave);
                connection = Some(rtu::attach_slave(port, slave));
                if connection.is_some() {
                    let _ = self
                        .status_sender
                        .send(Status::String(str!("Modbus TCP connected.")))
                        .await;
                    let _ = self
                        .log_sender
                        .send(LogMsg::ok(&format!(
                            "Modbus RTU connected successfully to {} with baud rate {}",
                            self.rtu_config.path, self.rtu_config.baud_rate
                        )))
                        .await;
                } else {
                    let _ = self
                        .log_sender
                        .send(LogMsg::err(&format!(
                            "Modbus RTU failed to connect to {} with baud rate {}",
                            self.rtu_config.path, self.rtu_config.baud_rate
                        )))
                        .await;
                }
                index = 0;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}
