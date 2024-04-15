//! High-level BLE communication library.
//!
//! The goal of this library is to provide an easy-to-use interface
//! for communicating with BLE devices, that satisfies most use cases.
//!
//! ## Usage
//!
//! Here is an example on how to find a device with battery level characteristic and read
//! a value from that characteristic:
//!
//! ```rust,no_run
//! use bleasy::common::characteristics::BATTERY_LEVEL;
//! use bleasy::{Error, ScanConfig, Scanner};
//! use futures::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Error> {
//!     pretty_env_logger::init();
//!
//!     // Create a filter for devices that have battery level characteristic
//!     let config = ScanConfig::default()
//!         .filter_by_characteristics(|uuids| uuids.contains(&BATTERY_LEVEL))
//!         .stop_after_first_match();
//!
//!     // Start scanning for devices
//!     let mut scanner = Scanner::new();
//!     scanner.start(config).await?;
//!
//!     // Take the first discovered device
//!     let device = scanner.device_stream().next().await.unwrap();
//!     println!("{:?}", device);
//!
//!     // Read the battery level
//!     let battery_level = device.characteristic(BATTERY_LEVEL).await?.unwrap();
//!     println!("Battery level: {:?}", battery_level.read().await?);
//!
//!     Ok(())
//! }
//!```

#![warn(clippy::all, future_incompatible, nonstandard_style, rust_2018_idioms)]

pub use btleplug::{api::BDAddr, Error, Result};

pub use characteristic::Characteristic;
pub use device::{Device, DeviceEvent};
pub use scanner::{ScanConfig, Scanner};

mod device;
mod scanner;

mod characteristic;
pub mod common;
