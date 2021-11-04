//! This example finds a BLE device with specified address.

use futures::StreamExt;
use bleasy::{Error, ScanConfig, Scanner};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let address = std::env::args().nth(1).expect("Expected address in format XX:XX:XX:XX:XX:XX");

    pretty_env_logger::init();

    log::info!("Scanning for device {}", address);

    let config = ScanConfig::default()
        .filter_by_address(move |addr| addr.to_string().eq(&address))
        .stop_after_first_match();

    let mut scanner = Scanner::new().await?;
    scanner.start(config).await?;

    let device = scanner.device_stream().next().await;

    println!("{:?}", device);

    Ok(())
}
