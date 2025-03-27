# Modbus CLI

Modbus CLI is a Rust implementation of a simple and intuative commandline application to interact with or simulate a modbus server. I started with the project because I'm used to work in environments that doesn't provide a GUI environment and my work required to handle different modbus use-cases.

If you prefer a GUI application, check out [QModbus](https://github.com/ed-chemnitz/qmodbus/) or similar applications.

## Goal

Provide a CLI application that can interact with a modbus server and modbus clients and visualize the status of modbus registers with live updates.

## Impressions

<p align="center">
    <p align="center">
        <img src="./img/modbus-cli-rs.png" style="border-radius: 8px">
    </p>
</p>

## Features

- [x] Modbus server that allows clients to manipulate the registers.
- [x] Modbus client to read and display all reigster contents of a modbus server.
- [x] Allow the manipulation of register contents in server and client mode.
- [x] Support TCP modbus
- [x] Support RTU modbus

## Quickstart

This project is written in Rust, thus you will have to install the rust toolchain to compile it. Just follow the instructions on [rustup.rs](https://rustup.rs/)
to set up the environment. Afterwards you are able to compile this project from source using the following command.

```sh
cargo build --release
```

Alternatively, you can also run it directly using the following command.

```sh
cargo run --release -- --config ./path/to/config.json tcp -i <ip> -p <port>
```

Please refer to `--help` for all available options.

## Configuration

The application will need a JSON configuration file. Besides some basic configuration parameters the configuration provides the register definitions.
These definitions are used to provide the table view and also group multiple registers together to e.g. limit read operations.

The configuraation has to contain the following entries.

```json
{
    "history_length": 30,
    "interval_ms": 500,
    "delay_after_connect_ms": 500,
    "contiguous_memory": [],
    "definitions": {}
}
```

The `history_length` defines the scroll back limit of the displayed log messages. The parameter `interval_ms` specifies the frequency of read operations
and is only used if the `modbus-cli-rs` application is executed in client mode using the `--client` flag. Based on the modbus server it may be necessary to
increase the duration if it can only handle a limited amount of commands per second.

In `contiguous_memory` you can define address ranges that are available on a modbus server. This is used to group multiple registers together and
reduce the amount of read commands. E.g. if you have two registers `0x200` and `0x202` and both registers have length 1, the client would perform
two read commands since `0x201` is unused and separates the two registers. By adding the following entry to `contiguous_memory`, you specify that
the range `[ 0x200, 0x202 ]` is provided by the modbus server and thus can be read using a single command without receiving a `Illigal Address`
exception. By default the `slave_id = 0` is used, if the modbus server has specific registers that are only available for a specific `slave_id`, you
can specify the specific `slave_id` for the memory range.

```json
"contiguous_memory": [
    {
        "slave_id": 1,
        "read_code": 4,
        "range": {
            "start": "0x200",
            "end": "0x202"
        }
    }
]
```

You can define all registers by adding the entries for each register to the `definitions` map. A definition entry looks like this. The `slave_id` is
here optional, too. If none is provided, `slave_id = 0` is used.

```json
"Serial Number": {
    "slave_id": 2,
    "read_code": 4,
    "address": "0x4000",
    "length": 4,
    "access": "ReadOnly",
    "type": "I32",
    "reverse": false
}
```

### Explanation:

- `slave_id`: The modbus slave identifier
- `read_code`: The modbus function code for read operation
- `address`: The modbus register address
- `length`: The successive modbus register count
- `access`: Accessability mode (either ReadOnly, WriteOnly or ReadWrite)
- `type`: The type that is represented by the modbus registers
- `reverse`: Define whether the registers have to be flipped before interpretation (default: `false`)

If you use the client mode `--client` the corresponding write codes for manipulating registers or coils are derived from the configured `read_code`. Please refer to `config.json` of this repository for a example configuration.

### Data Types

The following data types are currently supported and can be configured:

- `PackedAscii`: Every byte is an ASCII character (single register = 2 ASCII characters)
- `LooseAscii`: Every register contains only a single ASCII character
- `PackedUtf8`: All combined register values represent a valid UTF-8 string
- `LooseUtf8`: Every register is a single UTF-8 character
- `U8`: The register contains a single 8-bit unsigned value
- `U16`: The register contains a 16-bit unsigned value
- `U32`: The combined register contents contain a 32-bit unsigned value
- `U64`: The combined register contents contain a 32-bit unsigned value
- `U128`: The combined register contents contain a 32-bit unsigned value
- `U8`: The register contains a single 8-bit little-endian unsigned value
- `U16`: The register contains a 16-bit little-endian unsigned value
- `U32`: The combined register contents contain a 32-bit little-endian unsigned value
- `U64`: The combined register contents contain a 32-bit little-endian unsigned value
- `U128`: The combined register contents contain a 32-bit little-endian unsigned value
- `I8`: The register contains a single 8-bit signed value
- `I16`: The register contains a 16-bit signed value
- `I32`: The combined register contents contain a 32-bit signed value
- `I64`: The combined register contents contain a 32-bit signed value
- `I128`: The combined register contents contain a 32-bit signed value
- `I8`: The register contains a single 8-bit little-endian signed value
- `I16`: The register contains a 16-bit little-endian signed value
- `I32`: The combined register contents contain a 32-bit little-endian signed value
- `I64`: The combined register contents contain a 32-bit little-endian signed value
- `I128`: The combined register contents contain a 32-bit little-endian signed value
- `F32`: The combined register contents contain a 32-bit float value
- `F32le`: The combined register contents contain a 32-bit little-endian float value
- `F64`: The combined register contents contain a 64-bit float value
- `F64le`: The combined register contents contain a 64-bit little-endian float value
