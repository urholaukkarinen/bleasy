use bleasy::{Error, ScanConfig, Scanner};
use futures::StreamExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    // Create a new BLE device scanner
    let mut scanner = Scanner::new().await?;

    // Start the scanner with default configuration
    scanner.start(ScanConfig::default()).await?;

    // Create a stream that is provided with discovered devices
    let mut device_stream = scanner.device_stream();

    // Create a thread-safe counter
    let count = Arc::new(AtomicU32::new(0));

    // List devices in a separate thread as they are discovered
    let join_handle = {
        let count = count.clone();
        tokio::spawn(async move {
            while let Some(device) = device_stream.next().await {
                println!("{:?}", device);
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    // Wait until at least two devices are found
    while count.load(Ordering::SeqCst) < 2 {
        sleep(Duration::from_millis(100)).await;
    }

    // Stop the scanner after 2 devices are found
    scanner.stop().await?;

    join_handle.await.unwrap();

    Ok(())
}
