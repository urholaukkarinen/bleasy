# bleasy

[![Latest version](https://img.shields.io/crates/v/bleasy.svg)](https://crates.io/crates/bleasy)
[![Documentation](https://docs.rs/bleasy/badge.svg)](https://docs.rs/bleasy)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)

High-level BLE communication library for Rust.

The goal of this library is to provide an easy-to-use interface
for communicating with BLE devices, that satisfies most use cases.

## Usage

Here is an example on how to find a device with battery level characteristic and read
a value from that characteristic. For more examples, see the [examples](./examples) directory.
```rust
use bleasy::common::characteristics::BATTERY_LEVEL;
use bleasy::{Error, ScanConfig, Scanner};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    // Filter devices that have battery level characteristic
    let config = ScanConfig::default()
        .filter_by_characteristics(|uuids| uuids.contains(&BATTERY_LEVEL))
        .stop_after_first_match();

    // Start scanning for devices
    let mut scanner = Scanner::new();
    scanner.start(config).await?;

    // Take the first discovered device
    let device = scanner.device_stream().next().await.unwrap();

    // Read the battery level
    let battery_level = device.characteristic(BATTERY_LEVEL).await?.unwrap();
    println!("Battery level: {:?}", battery_level.read().await?);

    Ok(())
}
```

## Background

At the time of writing this, there is only one cross-platform BLE library for Rust, [btleplug](https://github.com/deviceplug/btleplug).
I have been using it for various personal projects for a while, but I found that I would like to have a bit higher-level library that
makes it quick and easy start working with BLE devices, something akin to [bleak](https://github.com/hbldh/bleak) for Python.

As this library in its current state uses btleplug, this can be seen as a sort of conveniency wrapper for it.