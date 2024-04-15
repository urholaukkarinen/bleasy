use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use btleplug::api::{BDAddr, Central, CentralEvent, Manager as _, Peripheral as _};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use btleplug::Error;
use futures::{Stream, StreamExt};
use uuid::Uuid;

use crate::{Device, DeviceEvent};
use stream_cancel::{Trigger, Valved};
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio_stream::wrappers::BroadcastStream;

#[derive(Default)]
pub struct ScanConfig {
    /// Index of the Bluetooth adapter to use. The first found adapter is used by default.
    adapter_index:          usize,
    /// Filters the found devices based on device address.
    address_filter:         Option<Box<dyn Fn(BDAddr) -> bool + Send + Sync>>,
    /// Filters the found devices based on local name.
    name_filter:            Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Filters the found devices based on characteristics. Requires a connection to the device.
    characteristics_filter: Option<Box<dyn Fn(&[Uuid]) -> bool + Send + Sync>>,
    /// Maximum results before the scan is stopped.
    max_results:            Option<usize>,
    /// The scan is stopped when timeout duration is reached.
    timeout:                Option<Duration>,
    /// Force disconnect when listen the device is connected.
    force_disconnect: bool,
}

impl ScanConfig {
    /// Index of bluetooth adapter to use
    #[inline]
    pub fn adapter_index(mut self, index: usize) -> Self {
        self.adapter_index = index;
        self
    }

    /// Filter scanned devices based on the device address
    #[inline]
    pub fn filter_by_address(mut self, func: impl Fn(BDAddr) -> bool + Send + Sync + 'static) -> Self {
        self.address_filter = Some(Box::new(func));
        self
    }

    /// Filter scanned devices based on the device name
    #[inline]
    pub fn filter_by_name(mut self, func: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.name_filter = Some(Box::new(func));
        self
    }

    /// Filter scanned devices based on available characteristics
    #[inline]
    pub fn filter_by_characteristics(
        mut self,
        func: impl Fn(&[Uuid]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.characteristics_filter = Some(Box::new(func));
        self
    }

    /// Stop the scan after given number of matches
    #[inline]
    pub fn stop_after_matches(mut self, max_results: usize) -> Self {
        self.max_results = Some(max_results);
        self
    }

    /// Stop the scan after the first match
    #[inline]
    pub fn stop_after_first_match(self) -> Self {
        self.stop_after_matches(1)
    }

    /// Stop the scan after given duration
    #[inline]
    pub fn stop_after_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[inline]
    pub fn force_disconnect(mut self, force_disconnect: bool) -> Self {
        self.force_disconnect = force_disconnect;
        self
    }

    /// Require that the scanned devices have a name
    #[inline]
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
    pub(crate) adapter:  Adapter,
}

pub struct Scanner {
    session:      Weak<Session>,
    event_sender: Sender<DeviceEvent>,
    stoppers:     Arc<RwLock<Vec<Trigger>>>,
    scan_stopper: Arc<AtomicBool>,
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        let (event_sender, _) = broadcast::channel(32);
        Self {
            scan_stopper: Arc::new(AtomicBool::new(false)),
            session:      Weak::new(),
            event_sender,
            stoppers:     Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Start scanning for ble devices.
    pub async fn start(&mut self, config: ScanConfig) -> Result<(), Error> {
        if self.session.upgrade().is_some() {
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
        self.session = Arc::downgrade(&session);

        let event_sender = self.event_sender.clone();

        let mut worker = ScannerWorker::new(
            config,
            session.clone(),
            event_sender,
            self.scan_stopper.clone());
        tokio::spawn( async move {
            worker.scan().await;
        });

        Ok(())
    }

    /// Stop scanning for ble devices.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.scan_stopper.store(true, Ordering::Relaxed);
        self.stoppers.write().unwrap().clear();
        log::info!("Scanner is stopped.");

        Ok(())
    }

    /// Returns true if the scanner is active.
    pub fn is_active(&self) -> bool {
        self.session.upgrade().is_some()
    }

    /// Create a new stream that receives ble device events.
    pub fn device_event_stream(
        &mut self,
    ) -> Valved<Pin<Box<dyn Stream<Item = DeviceEvent> + Send>>> {
        let receiver = self.event_sender.subscribe();

        let stream: Pin<Box<dyn Stream<Item = DeviceEvent> + Send>> =
            Box::pin(BroadcastStream::new(receiver).filter_map(|x| async move {
                match x {
                    Ok(event) => {
                        log::debug!("Broadcasting device: {:?}", event);
                        Some(event)

                    },
                    Err(e) => {
                        log::warn!("Error: {:?} when broadcasting device event!", e);
                        None
                    },
                }
            }));

        let (trigger, stream) = Valved::new(stream);
        self.stoppers.write().unwrap().push(trigger);

        stream
    }

    /// Create a new stream that receives discovered ble devices.
    pub fn device_stream(&mut self) -> Valved<Pin<Box<dyn Stream<Item = Device> + Send>>> {
        let receiver = self.event_sender.subscribe();

        let stream: Pin<Box<dyn Stream<Item = Device> + Send>> =
            Box::pin(BroadcastStream::new(receiver).filter_map(|x| async move {
                match x {
                    Ok(DeviceEvent::Discovered(device)) => {
                        log::debug!("Broadcasting device: {:?}", device.address());
                        Some(device)
                    },
                    Err(e) => {
                        log::warn!("Error: {:?} when broadcasting device!", e);
                        None
                    },
                    _ => {
                        log::warn!("Unknown error when broadcasting device!");
                        None
                    },
                }
            }));

        let (trigger, stream) = Valved::new(stream);
        self.stoppers.write().unwrap().push(trigger);

        stream
    }
}

pub struct ScannerWorker {
    /// Configurations for the scan, such as filters and stop conditions
    config:            ScanConfig,
    /// Reference to the bluetooth session instance
    session:           Arc<Session>,
    /// Number of matching devices found so far
    result_count:      usize,
    /// Whether a connection is needed in order to pass the filter
    connection_needed: bool,
    /// Set of devices that have been filtered and will be ignored
    filtered:          HashSet<PeripheralId>,
    /// Set of devices that we are currently connecting to
    connecting:        Arc<Mutex<HashSet<PeripheralId>>>,
    /// Set of devices that matched the filters
    matched:           HashSet<PeripheralId>,
    /// Channel for sending events to the client
    event_sender:      Sender<DeviceEvent>,
    /// Stop the scan event.
    stopper:           Arc<AtomicBool>,
}

impl ScannerWorker {

    fn new(
        config:       ScanConfig,
        session:      Arc<Session>,
        event_sender: Sender<DeviceEvent>,
        stopper:      Arc<AtomicBool>) -> Self {
        let connection_needed = config.characteristics_filter.is_some();

        Self {
            config,
            session,
            result_count: 0,
            connection_needed,
            filtered: HashSet::new(),
            connecting: Arc::new(Mutex::new(HashSet::new())),
            matched: HashSet::new(),
            event_sender,
            stopper,
        }
    }

    async fn scan(&mut self) {
        log::info!("Starting the scan");

        match self.session.adapter.start_scan(Default::default()).await {
            Ok(()) => {
                while let Ok(mut stream) = self.session.adapter.events().await {
                    let start_time = Instant::now();

                    while let Some(event) = stream.next().await {
                        match event {
                            CentralEvent::DeviceDiscovered(peripheral_id) => {
                                self.on_device_discovered(peripheral_id).await;
                            },
                            CentralEvent::DeviceUpdated(peripheral_id) => {
                                self.on_device_updated(peripheral_id).await;
                            },
                            CentralEvent::DeviceConnected(peripheral_id) => {
                                self.on_device_connected(peripheral_id).await;
                            },
                            CentralEvent::DeviceDisconnected(peripheral_id) => {
                                self.on_device_disconnected(peripheral_id).await;
                            },
                            _ => {},
                        }

                        let timeout_reached = self.config
                            .timeout
                            .filter(|timeout| Instant::now().duration_since(start_time).ge(timeout))
                            .is_some();

                        let max_result_reached = self.config
                            .max_results
                            .filter(|max_results| self.result_count >= *max_results)
                            .is_some();

                        if timeout_reached
                            || max_result_reached
                            || self.stopper.load(Ordering::Relaxed) {
                            log::info!("Scanner stop condition reached.");
                            return;
                        }
                    }
                }
            },
            Err(e) => log::warn!("Error: `{:?}` when start scan!", e),
        }
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
                let address = peripheral.address();
                match self.event_sender.send(DeviceEvent::Updated(Device::new(
                    self.session.adapter.clone(),
                    peripheral,
                ))) {
                    Ok(value) => log::debug!("Sent device: {}, size: {}...", address, value),
                    Err(e) => log::debug!("Error: {:?} when Sending device: {}...", e, address),
                }
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
                let address = peripheral.address();
                match self.event_sender.send(DeviceEvent::Connected(Device::new(
                    self.session.adapter.clone(),
                    peripheral,
                ))) {
                    Ok(value) => log::debug!("Sent device: {}, size: {}...", address, value),
                    Err(e) => log::debug!("Error: {:?} when Sending device: {}...", e, address),
                }
            } else {
                self.apply_filter(peripheral).await;
            }
        }
    }

    async fn on_device_disconnected(&mut self, peripheral_id: PeripheralId) {
        if let Ok(peripheral) = self.session.adapter.peripheral(&peripheral_id).await {
            log::trace!("Device disconnected: {:?}", peripheral);

            if self.matched.contains(&peripheral_id) {
                let address = peripheral.address();
                match self.event_sender.send(DeviceEvent::Disconnected(Device::new(
                    self.session.adapter.clone(),
                    peripheral,
                ))) {
                    Ok(value) => log::debug!("Sent device: {}, size: {}...", address, value),
                    Err(e) => log::debug!("Error: {:?} when Sending device: {}...", e, address),
                }
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
        } else if self.config.force_disconnect {
            peripheral.disconnect().await.ok();
        }

        self.add_peripheral(peripheral).await;
    }

    async fn skip_peripheral(&mut self, peripheral: &Peripheral) {
        self.filtered.insert(peripheral.id());

        if self.config.force_disconnect {
            peripheral.disconnect().await.ok();
            return;
        }

        if let Ok(connected) = peripheral.is_connected().await {
            if !connected {
                return;
            }
        }

        if let Some(filter_by_address) = self.config.address_filter.as_ref() {
            if let Ok(Some(property)) = peripheral.properties().await {
                if filter_by_address(property.address) {
                    peripheral.disconnect().await.ok();
                }
            };
        }

        if let Some(filter_by_name) = self.config.name_filter.as_ref() {
            if let Ok(Some(property)) = peripheral.properties().await {
                if let Some(local_name) = property.local_name {
                    if filter_by_name(local_name.as_str()) {
                        peripheral.disconnect().await.ok();
                    }
                }
            }
        }
    }

    async fn add_peripheral(&mut self, peripheral: Peripheral) {
        self.filtered.insert(peripheral.id());
        self.matched.insert(peripheral.id());

        let address = peripheral.address();
        log::info!("Found device: {:?}", address);

        match self.event_sender.send(DeviceEvent::Discovered(Device::new(
            self.session.adapter.clone(),
            peripheral,
        ))) {
            Ok(value) => {
                log::debug!("Sent device: {}, size: {}...", address, value);
                self.result_count += value;
            }
            Err(e) => log::error!("Error: `{:?}` when adding peripheral: {}", e, address),
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

            passed &= if characteristics.is_empty() {
                let address = peripheral.address();
                log::debug!("Discovering characteristics for {}", address);

                match peripheral.discover_services().await {
                    Ok(()) => {
                        characteristics.extend(peripheral.characteristics());
                        let characteristics = characteristics
                            .into_iter()
                            .map(|c| c.uuid)
                            .collect::<Vec<_>>();
                        filter_by_characteristics(characteristics.as_slice())
                    }
                    Err(e) => {
                        log::warn!(
                            "Error: `{:?}` when discovering characteristics for {}",
                            e,
                            address
                        );
                        false
                    }
                }
            } else {
                true
            }
        }

        Some(passed)
    }
}
