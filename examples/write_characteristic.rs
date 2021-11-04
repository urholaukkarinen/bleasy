//! This example powers on a SteamVR base station.
//! The device name should be given as a command line argument.

use bleasy::{Error, ScanConfig, Scanner};
use futures::StreamExt;
use std::str::FromStr;
use uuid::Uuid;

const POWER_UUID: &str = "00001525-1212-efde-1523-785feabcd124";

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Give BLE device address as a command line argument.

    let name = std::env::args().nth(1).expect("Expected device name");
    pretty_env_logger::init();

    let config = ScanConfig::default()
        .filter_by_name(move |n| n.eq(&name))
        .stop_after_first_match();

    let mut scanner = Scanner::new().await?;
    scanner.start(config).await?;

    let mut device_stream = scanner.device_stream();

    let device = device_stream.next().await.unwrap();

    let uuid = Uuid::from_str(POWER_UUID).unwrap();
    let power = device.characteristic(uuid).await?.unwrap();

    println!("Power: {:?}", power.read().await.unwrap());

    power.write_command(&[1]).await.unwrap();

    Ok(())
}
