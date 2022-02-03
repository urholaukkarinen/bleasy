//! This example listens for BLE device events.

use bleasy::{DeviceEvent, Error, ScanConfig, Scanner};
use futures::StreamExt;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    // Create a new BLE device scanner
    let mut scanner = Scanner::new();

    // Start the scanner with default configuration
    scanner.start(ScanConfig::default()).await?;

    // Create a stream that is provided with device events
    let mut event_stream = scanner.device_event_stream();

    // Read events in a separate thread
    let join_handle = tokio::spawn(async move {
        while let Some(event) = event_stream.next().await {
            match event {
                DeviceEvent::Discovered(device) => {
                    println!("Device discovered: {}", device.address())
                }
                DeviceEvent::Connected(device) => {
                    println!("Device connected: {}", device.address())
                }
                DeviceEvent::Disconnected(device) => {
                    println!("Device disconnected: {}", device.address())
                }
                DeviceEvent::Updated(device) => {
                    println!("Device updated: {}", device.address())
                }
            }
        }
    });

    sleep(Duration::from_millis(2000)).await;

    scanner.stop().await?;

    join_handle.await.unwrap();

    Ok(())
}
