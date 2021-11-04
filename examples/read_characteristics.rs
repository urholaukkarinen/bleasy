//! This example finds the first device with battery level characteristic
//! and reads the device's battery level.

use bleasy::common::characteristics::BATTERY_LEVEL;
use bleasy::{Error, ScanConfig, Scanner};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    // Filters devices that have battery level characteristic
    let config = ScanConfig::default()
        .filter_by_characteristics(|uuids| uuids.contains(&BATTERY_LEVEL))
        .stop_after_first_match();

    // Start scanning for devices
    let mut scanner = Scanner::new().await?;
    scanner.start(config).await?;

    // Take the first discovered device
    let device = scanner.device_stream().next().await.unwrap();
    println!("{:?}", device);

    // Read the battery level
    let battery_level = device.characteristic(BATTERY_LEVEL).await?.unwrap();
    println!("Battery level: {:?}", battery_level.read().await?);

    Ok(())
}
