use crate::mem::data::DataType;
use crate::mem::memory::{Memory, Range};
use crate::mem::register::{AccessType, Definition};
use crate::msg::LogMsg;
use crate::rtu::RtuConfig;
use crate::util::{str, Expect};
use crate::{AppConfig, Command, Status};

use itertools::Itertools;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_modbus::prelude::SlaveId;
use tokio_modbus::prelude::{rtu, Client as ModbusClient, Reader, Slave, SlaveContext, Writer};
use tokio_modbus::FunctionCode;
use tokio_serial::{DataBits, Parity, SerialPortBuilder, SerialStream, StopBits};

pub struct Client {
    config: RtuConfig,
    memory: Arc<Mutex<Memory>>,
    operations: Vec<(SlaveId, FunctionCode, Range<usize>)>,
    status_sender: Sender<Status>,
    cmd_receiver: Receiver<Command>,
    log_sender: Sender<LogMsg>,
}

impl Client {
    pub fn new(
        app_config: Arc<Mutex<AppConfig>>,
        rtu_config: RtuConfig,
        memory: Arc<Mutex<Memory>>,
        status_sender: Sender<Status>,
        cmd_receiver: Receiver<Command>,
        log_sender: Sender<LogMsg>,
    ) -> Self {
        let operations = Self::init(app_config);
        Self {
            config: rtu_config,
            memory,
            operations,
            status_sender,
            cmd_receiver,
            log_sender,
        }
    }

    fn init(app_config: Arc<Mutex<AppConfig>>) -> Vec<(SlaveId, FunctionCode, Range<usize>)> {
        let config = app_config.lock().expect("Unable to lock configuration");
        let mut sorted_defs = config
            .definitions
            .iter()
            .filter(|d| !d.1.is_virtual())
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
                None,
                0,
                0,
                DataType::default(),
                0,
                AccessType::ReadOnly,
                None,
                None,
                None,
                None,
            ),
        );
        sorted_defs.push((&marker.0, &marker.1));

        let is_allowed = |fc: u8, addr: u16, end: usize| {
            for mem in config.contiguous_memory.iter() {
                if mem.read_code == fc
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
                    def.get_slave_id(),
                    def.get_address(),
                    def.read_code(),
                    def.get_range(),
                );
                if (fc != -1 && fc != (def_fc as i16))
                    || (slave != (def_slave.unwrap_or(0) as i16))
                    || ((range.length() + def_addr as usize + def_range.length())
                        > (range.start() + 127))
                    || (def_addr >= range.end() as u16
                        && !is_allowed(fc as u8, def_addr, range.end()))
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

    async fn create_serial_builder(&self) -> SerialPortBuilder {
        let mut builder = tokio_serial::new(self.config.path.clone(), self.config.baud_rate);
        let data_bits = self.config.data_bits.unwrap_or(8);
        let stop_bits = self.config.stop_bits.unwrap_or(1);
        let parity = self
            .config
            .parity
            .as_ref()
            .unwrap_or(&"NONE".to_string())
            .to_uppercase();
        let flow_control = self
            .config
            .flow_control
            .as_ref()
            .unwrap_or(&crate::rtu::FlowControl::None);

        builder = builder.data_bits(match data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            8 => DataBits::Eight,
            _ => panic!("Invalid data bits specified."),
        });

        builder = builder.stop_bits(match stop_bits {
            1 => StopBits::One,
            2 => StopBits::Two,
            _ => panic!("Invalid stop bits specified"),
        });

        if parity == "ODD" {
            builder = builder.parity(Parity::Odd);
        } else if parity == "EVEN" {
            builder = builder.parity(Parity::Even);
        } else if parity == "NONE" {
            builder = builder.parity(Parity::None);
        } else {
            panic!("Invalid parity specified");
        }

        builder = builder.flow_control(match flow_control {
            crate::rtu::FlowControl::None => tokio_serial::FlowControl::None,
            crate::rtu::FlowControl::Software => tokio_serial::FlowControl::Software,
            crate::rtu::FlowControl::Hardware => tokio_serial::FlowControl::Hardware,
        });

        builder
    }

    fn config_as_str(&self) -> String {
        let path = &self.config.path;
        let baud_rate = self.config.baud_rate;
        let data_bits = self.config.data_bits.unwrap_or(8);
        let stop_bits = self.config.stop_bits.unwrap_or(1);
        let parity = self
            .config
            .parity
            .as_ref()
            .unwrap_or(&"NONE".to_string())
            .to_uppercase();
        let flow_control = self
            .config
            .flow_control
            .as_ref()
            .unwrap_or(&crate::rtu::FlowControl::None);
        format!(
            "{}, baud rate: {}, data bits: {}, parity: {}, stop bits: {}, flow control: {}",
            path, baud_rate, data_bits, parity, stop_bits, flow_control
        )
    }

    pub async fn run(&mut self, delay_after_connect: u64, interval_ms: u64, timeout_ms: u64) {
        let builder = self.create_serial_builder().await;
        let port =
            SerialStream::open(&builder).panic(|e| format!("Failed to open SerialStream ({e})"));
        let slave = Slave(self.config.client_id);
        let mut connection = Some(rtu::attach_slave(port, slave));
        if connection.is_some() {
            let _ = self
                .status_sender
                .send(Status::String(str!("Modbus TCP connected.")))
                .await;
            let _ = self
                .log_sender
                .send(LogMsg::ok(&format!(
                    "Modbus RTU connected to {}",
                    self.config_as_str()
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
                    "Modbus TCP failed to connect to {}",
                    self.config_as_str()
                )))
                .await;
        };

        if delay_after_connect > 0 {
            let _ = self
                .log_sender
                .send(LogMsg::info(&format!(
                    "Wait for {}ms after connect",
                    delay_after_connect
                )))
                .await;
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_after_connect)).await;
        }

        let mut time_last_read = SystemTime::now()
            .checked_sub(Duration::from_millis(interval_ms + 1))
            .expect("Unable to calculate time difference");
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
                    let (slave, fc, op) = self
                        .operations
                        .get(op_idx)
                        .expect("Unable to retrieve operation");
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
                                end = op.end()
                            )))
                            .await;
                        let mut memory = self.memory.lock().expect("Unable to lock memory");
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

                        let err = match modbus_result {
                            Err(_) => str!("Timeout elapsed"),
                            Ok(Err(e)) => format!("{}", e),
                            Ok(Ok(Err(e))) => format!("Exception {}", e),
                            _ => str!(""),
                        };

                        let _ = self.log_sender
                            .send(LogMsg::err(&format!(
                                "Read address space [ {start:#06X} ({start}), {end:#06X} ({end}) ) failed ({err}).",
                                start = op.start(),
                                end = op.end(),
                                err = err
                            )))
                            .await;
                        let _ = self
                            .status_sender
                            .send(Status::String(str!("Modbus RTU disconnected.")))
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
                                .send(Status::String(str!("Modbus RTU disconnected.")))
                                .await;
                            let _ = self
                                .log_sender
                                .send(LogMsg::ok(&format!(
                                    "Modbus RTU disconnected from {}",
                                    self.config_as_str()
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
                    let builder = self.create_serial_builder().await;
                    let port = SerialStream::open(&builder)
                        .panic(|e| format!("Failed to open SerialStream ({e})"));
                    let slave = Slave(self.config.client_id);
                    connection = Some(rtu::attach_slave(port, slave));
                    if connection.is_some() {
                        let _ = self
                            .status_sender
                            .send(Status::String(str!("Modbus TCP connected.")))
                            .await;
                        let _ = self
                            .log_sender
                            .send(LogMsg::ok(&format!(
                                "Modbus RTU reconnected successfully to {}",
                                self.config_as_str()
                            )))
                            .await;
                        if delay_after_connect > 0 {
                            let _ = self
                                .log_sender
                                .send(LogMsg::info(&format!(
                                    "Wait for {}ms after reconnect",
                                    delay_after_connect
                                )))
                                .await;
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                delay_after_connect,
                            ))
                            .await;
                        }
                    } else {
                        let _ = self
                            .log_sender
                            .send(LogMsg::err(&format!(
                                "Modbus RTU failed to reconnect to {}",
                                self.config_as_str()
                            )))
                            .await;
                    }
                }
            } else if let Ok(Command::Connect) = self.cmd_receiver.try_recv() {
                let builder = self.create_serial_builder().await;
                let port = SerialStream::open(&builder)
                    .panic(|e| format!("Failed to open SerialStream ({e})"));
                let slave = Slave(self.config.client_id);
                connection = Some(rtu::attach_slave(port, slave));
                if connection.is_some() {
                    let _ = self
                        .status_sender
                        .send(Status::String(str!("Modbus TCP connected.")))
                        .await;
                    let _ = self
                        .log_sender
                        .send(LogMsg::ok(&format!(
                            "Modbus RTU connected successfully to {}",
                            self.config_as_str()
                        )))
                        .await;
                    if delay_after_connect > 0 {
                        let _ = self
                            .log_sender
                            .send(LogMsg::info(&format!(
                                "Wait for {}ms after connect",
                                delay_after_connect
                            )))
                            .await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_after_connect))
                            .await;
                    }
                } else {
                    let _ = self
                        .log_sender
                        .send(LogMsg::err(&format!(
                            "Modbus RTU failed to connect to {}",
                            self.config_as_str()
                        )))
                        .await;
                }
                op_idx = 0;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}
