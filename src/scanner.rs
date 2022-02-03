use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use btleplug::api::{BDAddr, Central, CentralEvent, Manager as _, Peripheral as _};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use btleplug::Error;
use futures::{Stream, StreamExt};
use uuid::Uuid;

use crate::Device;
use stream_cancel::{Trigger, Valved};
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio_stream::wrappers::BroadcastStream;

#[derive(Default)]
pub struct ScanConfig {
    /// Index of the Bluetooth adapter to use. The first found adapter is used by default.
    adapter_index: usize,
    /// Filters the found devices based on device address.
    address_filter: Option<Box<dyn Fn(BDAddr) -> bool + Send>>,
    /// Filters the found devices based on local name.
    name_filter: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Filters the found devices based on characteristics. Requires a connection to the device.
    characteristics_filter: Option<Box<dyn Fn(&[Uuid]) -> bool + Send + Sync>>,
    /// Maximum results before the scan is stopped.
    max_results: Option<usize>,
    /// The scan is stopped when timeout duration is reached.
    timeout: Option<Duration>,
}

impl ScanConfig {
    /// Index of bluetooth adapter to use
    pub fn adapter_index(mut self, index: usize) -> Self {
        self.adapter_index = index;
        self
    }

    /// Filter scanned devices based on the device address
    pub fn filter_by_address(mut self, func: impl Fn(BDAddr) -> bool + Send + 'static) -> Self {
        self.address_filter = Some(Box::new(func));
        self
    }

    /// Filter scanned devices based on the device name
    pub fn filter_by_name(mut self, func: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.name_filter = Some(Box::new(func));
        self
    }

    /// Filter scanned devices based on available characteristics
    pub fn filter_by_characteristics(
        mut self,
        func: impl Fn(&[Uuid]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.characteristics_filter = Some(Box::new(func));
        self
    }

    /// Stop the scan after given number of matches
    pub fn stop_after_matches(mut self, max_results: usize) -> Self {
        self.max_results = Some(max_results);
        self
    }

    /// Stop the scan after the first match
    pub fn stop_after_first_match(self) -> Self {
        self.stop_after_matches(1)
    }

    /// Stop the scan after given duration
    pub fn stop_after_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Require that the scanned devices have a name
    pub fn require_name(self) -> Self {
        if self.name_filter.is_none() {
            self.filter_by_name(|name| !name.is_empty())
        } else {
            self
        }
    }
}

pub(crate) struct Session {
    pub(crate) _manager: Manager,
    pub(crate) adapter: Adapter,
}

pub struct Scanner {
    session: Option<Arc<Session>>,
    event_sender: Sender<DeviceEvent>,
    scan_stopper: Option<Trigger>,
    device_stream_stoppers: Arc<RwLock<Vec<Trigger>>>,
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        let (event_sender, _) = broadcast::channel(16);

        Self {
            session: None,
            event_sender,
            scan_stopper: None,
            device_stream_stoppers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Start scanning for ble devices.
    pub async fn start(&mut self, config: ScanConfig) -> Result<(), Error> {
        if self.session.is_some() {
            log::info!("Scanner is already started.");
            return Ok(());
        }

        let manager = Manager::new().await?;
        let mut adapters = manager.adapters().await?;

        if config.adapter_index >= adapters.len() {
            return Err(Error::DeviceNotFound);
        }

        let adapter = adapters.swap_remove(config.adapter_index);

        log::trace!("Using adapter: {:?}", adapter);

        let session = Arc::new(Session {
            _manager: manager,
            adapter,
        });
        let stopper = ScanContext::start(
            config,
            session.clone(),
            self.event_sender.clone(),
            self.device_stream_stoppers.clone(),
        )
        .await?;

        self.scan_stopper = Some(stopper);
        self.session = Some(session);

        Ok(())
    }

    /// Stop scanning for ble devices.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(session) = self.session.take() {
            session.adapter.stop_scan().await?;
            self.scan_stopper.take();
            self.device_stream_stoppers.write().unwrap().clear();
        } else {
            log::info!("Scanner is already stopped");
        }

        Ok(())
    }

    /// Create a new stream that receives ble device events.
    pub fn device_event_stream(
        &mut self,
    ) -> Valved<Pin<Box<dyn Stream<Item = DeviceEvent> + Send>>> {
        let receiver = self.event_sender.subscribe();

        let stream: Pin<Box<dyn Stream<Item = DeviceEvent> + Send>> =
            Box::pin(BroadcastStream::new(receiver).filter_map(|x| async move { x.ok() }));

        let (trigger, stream) = Valved::new(stream);
        self.device_stream_stoppers.write().unwrap().push(trigger);

        stream
    }

    /// Create a new stream that receives discovered ble devices.
    pub fn device_stream(&mut self) -> Valved<Pin<Box<dyn Stream<Item = Device> + Send>>> {
        let receiver = self.event_sender.subscribe();

        let stream: Pin<Box<dyn Stream<Item = Device> + Send>> =
            Box::pin(BroadcastStream::new(receiver).filter_map(|x| async move {
                match x {
                    Ok(DeviceEvent::Discovered(device)) => Some(device),
                    _ => None,
                }
            }));

        let (trigger, stream) = Valved::new(stream);
        self.device_stream_stoppers.write().unwrap().push(trigger);

        stream
    }
}

struct ScanContext {
    /// Number of matching devices found so far
    result_count: usize,
    /// Reference to the bluetooth session instance
    session: Arc<Session>,
    /// Configurations for the scan, such as filters and stop conditions
    config: ScanConfig,
    /// Whether a connection is needed in order to pass the filter
    connection_needed: bool,
    /// Set of devices that have been filtered and will be ignored
    filtered: HashSet<PeripheralId>,
    /// Set of devices that we are currently connecting to
    connecting: Arc<Mutex<HashSet<PeripheralId>>>,
    /// Set of devices that matched the filters
    matched: HashSet<PeripheralId>,
    /// Channel for sending events to the client
    event_sender: Sender<DeviceEvent>,
}

impl ScanContext {
    async fn start(
        config: ScanConfig,
        session: Arc<Session>,
        sender: Sender<DeviceEvent>,
        device_stream_stoppers: Arc<RwLock<Vec<Trigger>>>,
    ) -> Result<Trigger, Error> {
        let connection_needed = config.characteristics_filter.is_some();

        log::info!("Starting the scan");

        let (stopper, events) = stream_cancel::Valved::new(session.adapter.events().await?);

        session.adapter.start_scan(Default::default()).await?;

        let ctx = ScanContext {
            result_count: 0,
            session,
            config,
            connection_needed,
            filtered: HashSet::new(),
            connecting: Arc::new(Mutex::new(HashSet::new())),
            matched: HashSet::new(),
            event_sender: sender,
        };

        tokio::spawn(async move {
            ctx.listen(events, device_stream_stoppers).await;
        });

        Ok(stopper)
    }

    async fn listen(
        mut self,
        mut event_stream: Valved<Pin<Box<dyn Stream<Item = CentralEvent> + Send>>>,
        device_stream_stoppers: Arc<RwLock<Vec<Trigger>>>,
    ) {
        let start_time = Instant::now();

        while let Some(event) = event_stream.next().await {
            match event {
                CentralEvent::DeviceDiscovered(peripheral_id) => {
                    self.on_device_discovered(peripheral_id).await;
                }
                CentralEvent::DeviceConnected(peripheral_id) => {
                    self.on_device_connected(peripheral_id).await;
                }
                CentralEvent::DeviceDisconnected(peripheral_id) => {
                    self.on_device_disconnected(peripheral_id).await;
                }
                CentralEvent::DeviceUpdated(peripheral_id) => {
                    self.on_device_updated(peripheral_id).await;
                }
                _ => {}
            }

            let timeout_reached = self
                .config
                .timeout
                .filter(|timeout| Instant::now().duration_since(start_time).ge(timeout))
                .is_some();
            let max_result_reached = self
                .config
                .max_results
                .filter(|max_results| self.result_count >= *max_results)
                .is_some();

            if timeout_reached || max_result_reached {
                log::info!("Scanner stop condition reached.");
                break;
            }
        }

        device_stream_stoppers.write().unwrap().clear();

        log::info!("Scanner was stopped.");
    }

    async fn on_device_discovered(&mut self, peripheral_id: PeripheralId) {
        if let Ok(peripheral) = self.session.adapter.peripheral(&peripheral_id).await {
            log::trace!("Device discovered: {:?}", peripheral);

            self.apply_filter(peripheral).await;
        }
    }

    async fn on_device_updated(&mut self, peripheral_id: PeripheralId) {
        if let Ok(peripheral) = self.session.adapter.peripheral(&peripheral_id).await {
            log::trace!("Device updated: {:?}", peripheral);

            if self.matched.contains(&peripheral_id) {
                self.event_sender
                    .send(DeviceEvent::Updated(Device::new(
                        self.session.clone(),
                        peripheral,
                    )))
                    .ok();
            } else {
                self.apply_filter(peripheral).await;
            }
        }
    }

    async fn on_device_connected(&mut self, peripheral_id: PeripheralId) {
        self.connecting.lock().unwrap().remove(&peripheral_id);

        if let Ok(peripheral) = self.session.adapter.peripheral(&peripheral_id).await {
            log::trace!("Device connected: {:?}", peripheral);

            if self.matched.contains(&peripheral_id) {
                self.event_sender
                    .send(DeviceEvent::Connected(Device::new(
                        self.session.clone(),
                        peripheral,
                    )))
                    .ok();
            } else {
                self.apply_filter(peripheral).await;
            }
        }
    }

    async fn on_device_disconnected(&mut self, peripheral_id: PeripheralId) {
        if let Ok(peripheral) = self.session.adapter.peripheral(&peripheral_id).await {
            log::trace!("Device disconnected: {:?}", peripheral);

            if self.matched.contains(&peripheral_id) {
                self.event_sender
                    .send(DeviceEvent::Disconnected(Device::new(
                        self.session.clone(),
                        peripheral,
                    )))
                    .ok();
            }
        }

        self.connecting.lock().unwrap().remove(&peripheral_id);
    }

    async fn apply_filter(&mut self, peripheral: Peripheral) {
        if self.filtered.contains(&peripheral.id()) {
            // The device has already been filtered.
            return;
        }

        match self.passes_pre_connect_filters(&peripheral).await {
            Some(false) => {
                self.skip_peripheral(&peripheral).await;
                return;
            }
            None => {
                // Could not yet check all of the filters
                return;
            }
            _ => {
                // All passed. Keep going.
            }
        };

        if self.connection_needed {
            if !peripheral.is_connected().await.unwrap_or(false) {
                if self.connecting.lock().unwrap().insert(peripheral.id()) {
                    log::debug!("Connecting to device {}", peripheral.address());

                    // Connect in another thread, so we can keep filtering other devices meanwhile.
                    let peripheral_clone = peripheral.clone();
                    let connecting_map = self.connecting.clone();
                    tokio::spawn(async move {
                        if let Err(e) = peripheral_clone.connect().await {
                            log::warn!(
                                "Could not connect to {}: {:?}",
                                peripheral_clone.address(),
                                e
                            );

                            connecting_map
                                .lock()
                                .unwrap()
                                .remove(&peripheral_clone.id());
                        };
                    });
                }
                return;
            } else if let Some(false) = self.passes_post_connect_filters(&peripheral).await {
                self.skip_peripheral(&peripheral).await;
                return;
            }
        }

        self.add_peripheral(peripheral).await;
    }

    async fn skip_peripheral(&mut self, peripheral: &Peripheral) {
        self.filtered.insert(peripheral.id());
        peripheral.disconnect().await.ok();
    }

    async fn add_peripheral(&mut self, peripheral: Peripheral) {
        self.filtered.insert(peripheral.id());
        self.matched.insert(peripheral.id());

        log::info!("Found device: {:?}", peripheral);

        let device = Device::new(self.session.clone(), peripheral);

        match self.event_sender.send(DeviceEvent::Discovered(device)) {
            Ok(_) => {
                self.result_count += 1;
            }
            Err(e) => log::error!("Failed to add device: {}", e),
        }
    }

    /// Checks if the peripheral passes all of the filters that
    /// do not require a connection to the device.
    async fn passes_pre_connect_filters(&mut self, peripheral: &Peripheral) -> Option<bool> {
        let mut passed = true;

        if let Some(filter_by_addr) = self.config.address_filter.as_ref() {
            passed &= filter_by_addr(peripheral.address());
        }

        if let Some(filter_by_name) = self.config.name_filter.as_ref() {
            passed &= match peripheral.properties().await {
                Ok(Some(props)) => props.local_name.map(|name| filter_by_name(&name)),
                _ => None,
            }?;
        }

        Some(passed)
    }

    /// Checks if the peripheral passes all of the filters that
    /// require a connection to the device.
    async fn passes_post_connect_filters(&mut self, peripheral: &Peripheral) -> Option<bool> {
        let mut passed = true;

        if !peripheral.is_connected().await.unwrap_or(false) {
            return None;
        }

        if let Some(filter_by_characteristics) = self.config.characteristics_filter.as_ref() {
            let mut characteristics = Vec::new();
            characteristics.extend(peripheral.characteristics());

            if characteristics.is_empty() {
                log::debug!("Discovering characteristics for {}", peripheral.address());
                // TODO: handle errors
                peripheral.discover_services().await.ok();
                characteristics.extend(peripheral.characteristics());
            }

            passed &= if characteristics.is_empty() {
                false
            } else {
                let characteristics = characteristics
                    .into_iter()
                    .map(|c| c.uuid)
                    .collect::<Vec<_>>();
                filter_by_characteristics(characteristics.as_slice())
            }
        }

        Some(passed)
    }
}

#[derive(Clone)]
pub enum DeviceEvent {
    Discovered(Device),
    Connected(Device),
    Disconnected(Device),
    Updated(Device),
}
