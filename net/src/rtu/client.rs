use crate::rtu::Config;
use crate::{Command, Error, Key, ModbusError, Operation, SerialError};

use memory::{Memory, Type};
use tokio::task::JoinHandle;

use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::sync::mpsc::Receiver;
use tokio::time::sleep;
use tokio_modbus::FunctionCode;
use tokio_modbus::client::Context as ClientContext;
use tokio_modbus::prelude::{Client as ModbusClient, Reader, Slave, SlaveContext, Writer, rtu};
use tokio_serial::{DataBits, Parity, SerialStream, StopBits};

pub struct ClientBuilder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    id: T,
    config: Arc<RwLock<Config>>,
    operations: Arc<RwLock<Vec<Operation>>>,
    memory: Arc<RwLock<Memory<Key<T>>>>,
}

impl<T> ClientBuilder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub fn new(
        id: T,
        config: Arc<RwLock<Config>>,
        operations: Arc<RwLock<Vec<Operation>>>,
        memory: Arc<RwLock<Memory<Key<T>>>>,
    ) -> Self {
        Self {
            id,
            config,
            operations,
            memory,
        }
    }

    pub async fn spawn<L, S>(
        &self,
        receiver: Receiver<Command>,
        log: L,
        status: S,
    ) -> Result<JoinHandle<Result<(), Error>>, Error>
    where
        L: AsyncFn(String) -> () + Send + Sync + 'static,
        S: AsyncFn(String) -> () + Send + Sync + 'static,
        for<'a> L::CallRefFuture<'a>: Send,
        for<'a> S::CallRefFuture<'a>: Send,
    {
        let guard = self.config.read().await;
        let client = Client::connect(&guard).await?;
        let operations = self.operations.clone();
        let memory = self.memory.clone();
        let timeout_ms = guard.timeout_ms;
        let delay_ms = guard.delay_ms;
        let interval_ms = guard.interval_ms;
        let id = self.id.clone();
        Ok(tokio::task::spawn(async move {
            client
                .run(
                    id,
                    operations,
                    memory,
                    receiver,
                    log,
                    status,
                    timeout_ms,
                    delay_ms,
                    interval_ms,
                )
                .await
        }))
    }
}

pub struct Client {
    context: ClientContext,
}

impl Client {
    pub async fn connect(config: &Config) -> Result<Self, Error> {
        let mut builder = tokio_serial::new(&config.path, config.baud_rate);
        if let Some(v) = config.data_bits {
            builder = builder.data_bits(match v {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                8 => DataBits::Eight,
                _ => {
                    return Err(SerialError::Configuration(
                        "Invalid data bits specified".to_string(),
                    )
                    .into());
                }
            });
        }
        if let Some(v) = config.stop_bits {
            builder = builder.stop_bits(match v {
                1 => StopBits::One,
                2 => StopBits::Two,
                _ => {
                    return Err(SerialError::Configuration(
                        "Invalid stop bits specified".to_string(),
                    )
                    .into());
                }
            });
        }
        if let Some(ref v) = config.parity {
            let v = v.to_lowercase();
            if v == "odd" {
                builder = builder.parity(Parity::Odd);
            } else if v == "even" {
                builder = builder.parity(Parity::Even);
            } else if v == "none" {
                builder = builder.parity(Parity::None);
            } else {
                return Err(
                    SerialError::Configuration("Invalid parity specified".to_string()).into(),
                );
            }
        }

        match SerialStream::open(&builder).map(|s| rtu::attach_slave(s, Slave(config.slave))) {
            Ok(context) => Ok(Self { context }),
            Err(e) => Err(SerialError::Error(e).into()),
        }
    }

    async fn read<L>(
        &mut self,
        op: &Operation,
        timeout_ms: usize,
        log: &L,
    ) -> (&'static str, Result<Vec<u16>, ModbusError>)
    where
        L: AsyncFn(String) -> () + Send + 'static,
        for<'a> L::CallRefFuture<'a>: Send,
    {
        let result = match op.fn_code {
            FunctionCode::ReadCoils => {
                (log)(format!(
                    "Perform ReadCoils request for slave ID {} and range [{}, {})",
                    op.slave_id, op.range.start, op.range.end,
                ))
                .await;
                self.context.set_slave(Slave(op.slave_id));
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.context.read_coils(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await
                .map(|r| {
                    r.map(|v| v.map(|b| b.into_iter().map(|e| if e { 1 } else { 0 }).collect()))
                });
                ("ReadCoils", res)
            }
            FunctionCode::ReadDiscreteInputs => {
                (log)(format!(
                    "Perform ReadDiscreteInputs request for slave ID {} and range [{}, {})",
                    op.slave_id, op.range.start, op.range.end,
                ))
                .await;
                self.context.set_slave(Slave(op.slave_id));
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.context.read_discrete_inputs(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await
                .map(|r| {
                    r.map(|v| v.map(|b| b.into_iter().map(|e| if e { 1 } else { 0 }).collect()))
                });
                ("ReadDiscreteInputs", res)
            }
            FunctionCode::ReadInputRegisters => {
                (log)(format!(
                    "Perform ReadInputRegisters request for slave ID {} and range [{}, {})",
                    op.slave_id, op.range.start, op.range.end,
                ))
                .await;
                self.context.set_slave(Slave(op.slave_id));
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.context.read_input_registers(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await;
                ("ReadInputRegisters", res)
            }
            FunctionCode::ReadHoldingRegisters => {
                (log)(format!(
                    "Perform ReadHoldingRegisters request for slave ID {} and range [{}, {})",
                    op.slave_id, op.range.start, op.range.end,
                ))
                .await;
                self.context.set_slave(Slave(op.slave_id));
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.context.read_holding_registers(
                        op.range.start as u16,
                        (op.range.end - op.range.start) as u16,
                    ),
                )
                .await;
                ("ReadHoldingRegisters", res)
            }
            _ => panic!("Invalid function code in operation."),
        };
        match result {
            (s, Ok(Ok(Ok(v)))) => (s, Ok(v)),
            (s, Ok(Ok(Err(e)))) => (s, Err(ModbusError::Exception(e))),
            (s, Ok(Err(e))) => (s, Err(ModbusError::Error(e))),
            (s, Err(e)) => (s, Err(ModbusError::Timeout(e))),
        }
    }

    pub fn interval_elapsed(&self, since: &mut Option<Instant>, interval_ms: usize) -> bool {
        let now = Instant::now();
        match since {
            Some(time) => {
                let duration = now.duration_since(*time);
                if duration.as_millis() > interval_ms as u128 {
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

    pub async fn run<T, L, S>(
        mut self,
        id: T,
        operations: Arc<RwLock<Vec<Operation>>>,
        memory: Arc<RwLock<Memory<Key<T>>>>,
        mut receiver: Receiver<Command>,
        log: L,
        status: S,
        timeout_ms: usize,
        delay_ms: usize,
        interval_ms: usize,
    ) -> Result<(), Error>
    where
        T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
        L: AsyncFn(String) -> () + Send + 'static,
        S: AsyncFn(String) -> () + Send + 'static,
        for<'a> L::CallRefFuture<'a>: Send,
        for<'a> S::CallRefFuture<'a>: Send,
    {
        let mut time: Option<Instant> = None;

        // Wait timeout until first operation
        sleep(Duration::from_millis(delay_ms as u64)).await;

        let mut index = 0;
        let mut retries = 0;
        loop {
            // Perform next read of registers
            if self.interval_elapsed(&mut time, interval_ms) {
                let operations = operations.read().await;
                let count = operations.len();
                if index >= count {
                    index = 0;
                }
                let operation = operations.get(index).map(|v| (*v).clone());

                if let Some(operation) = operation {
                    let fc = operation.fn_code;
                    let range = operation.range.clone();
                    let start = range.start;
                    let end = range.end;
                    match self.read(&operation, timeout_ms, &log).await {
                        (s, Ok(values)) => {
                            let fc = FunctionCode::from(fc);
                            let mut guard = memory.write().await;
                            let key = Key {
                                id: id.clone(),
                                slave_id: operation.slave_id,
                            };
                            let ty = if fc == FunctionCode::ReadDiscreteInputs
                                || fc == FunctionCode::ReadHoldingRegisters
                            {
                                Type::Register
                            } else {
                                Type::Coil
                            };
                            if !guard.write(key, &ty, &range, &values) {
                                log(format!("{s} Failed because of failing memory update for [{start}, {end})."))
                                    .await;
                            }
                            index = (index + 1) % count;
                            retries = 0;
                        }
                        (s, Err(ModbusError::Timeout(e))) => {
                            let _ = self.context.disconnect().await;
                            log(format!(
                                    "{s} request to read [{start}, {end}) timed out. Disconnecting client. [{e:?}]"
                                )).await;
                            return Err(ModbusError::Timeout(e).into());
                        }
                        (s, Err(ModbusError::Error(e))) => {
                            let _ = self.context.disconnect().await;
                            log(format!(
                                    "{s} request to read [{start}, {end}) failed. Disconnecting client. [{e:?}]"
                                )).await;
                            return Err(ModbusError::Error(e).into());
                        }
                        (s, Err(ModbusError::Exception(e))) => {
                            retries += 1;
                            if retries > 3 {
                                log(format!(
                                    "{s} request to read [{start}, {end}) invalid. [{e}]"
                                ))
                                .await;
                                index = (index + 1) % count;
                                retries = 0;
                            }
                        }
                    }
                }
            }

            // Execute next command if available
            if let Ok(cmd) = receiver.try_recv() {
                match cmd {
                    Command::Terminate => {
                        let _ = self.context.disconnect().await;
                        log("Client gracefully terminated.".to_string()).await;
                        status("Client disconnected".to_string()).await;
                        return Ok(());
                    }
                    Command::WriteSingleCoil(slave, addr, coil) => {
                        self.context.set_slave(Slave(slave));
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms as u64),
                            self.context.write_single_coil(addr, coil),
                        )
                        .await
                        {
                            Err(e) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteSingleCoil request to {addr} with {coil} timed out. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Timeout(e).into());
                            }
                            Ok(Err(e)) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteSingleCoil request to {addr} with {coil} failed. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Error(e).into());
                            }
                            Ok(Ok(Err(e))) => {
                                log(format!(
                                    "WriteSingleCoil request to {addr} with {coil} invalid. [{e:?}]"
                                ))
                                .await;
                            }
                            Ok(Ok(Ok(_))) => {
                                log(format!(
                                    "WriteSingleCoil request to {addr} with {coil} successfully executed."
                                )).await;
                            }
                        }
                    }
                    Command::WriteMultipleCoils(slave, addr, coils) => {
                        self.context.set_slave(Slave(slave));
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms as u64),
                            self.context.write_multiple_coils(addr, &coils),
                        )
                        .await
                        {
                            Err(e) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteMultipleCoils request to {addr} with {coils:?} timed out. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Timeout(e).into());
                            }
                            Ok(Err(e)) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteMultipleCoils request to {addr} with {coils:?} failed. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Error(e).into());
                            }
                            Ok(Ok(Err(e))) => {
                                log(format!(
                                    "WriteMultipleCoils request to {addr} with {coils:?} failed. [{e:?}]"
                                )).await;
                            }
                            Ok(_) => {
                                log(format!(
                                    "WriteMultipleCoils request to {addr} with {coils:?} successfully executed."
                                )).await;
                            }
                        }
                    }
                    Command::WriteSingleRegister(slave, addr, value) => {
                        self.context.set_slave(Slave(slave));
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms as u64),
                            self.context.write_single_register(addr, value),
                        )
                        .await
                        {
                            Err(e) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteSingleRegister request to {addr} with {value} timed out. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Timeout(e).into());
                            }
                            Ok(Err(e)) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteSingleRegister request to {addr} with {value} failed. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Error(e).into());
                            }
                            Ok(Ok(Err(e))) => {
                                log(format!(
                                    "WriteSingleRegister request to {addr} with {value} invalid. [{e:?}]"
                                )).await;
                            }
                            Ok(_) => {
                                log(format!(
                                    "WriteSingleRegister request to {addr} with {value} successfully executed."
                                )).await;
                            }
                        }
                    }
                    Command::WriteMultipleRegister(slave, addr, values) => {
                        self.context.set_slave(Slave(slave));
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(timeout_ms as u64),
                            self.context.write_multiple_registers(addr, &values),
                        )
                        .await
                        {
                            Err(e) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteMultipleRegister request to {addr} with {values:?} timed out. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Timeout(e).into());
                            }
                            Ok(Err(e)) => {
                                let _ = self.context.disconnect().await;
                                log(format!(
                                    "WriteMultipleRegister request to {addr} with {values:?} failed. Disconnecting client. [{e:?}]"
                                )).await;
                                return Err(ModbusError::Error(e).into());
                            }
                            Ok(Ok(Err(e))) => {
                                log(format!(
                                    "WriteMultipleRegister request to {addr} with {values:?} invalid. [{e:?}]"
                                )).await;
                            }
                            Ok(_) => {
                                log(format!(
                                    "WriteMultipleRegister request to {addr} with {values:?} successfully executed."
                                )).await;
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }
}
