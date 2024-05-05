use crate::Characteristic;
use btleplug::{
    api::{
        BDAddr, Peripheral as _,
    },
    platform::{Adapter, Peripheral},
    Result,
};
use btleplug::api::Service;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Device {
    pub(self) _adapter:    Adapter,
    pub(crate) peripheral: Peripheral,
}

impl Device {
    pub(crate) fn new(adapter: Adapter, peripheral: Peripheral) -> Self {
        Self {
            _adapter: adapter,
            peripheral,
        }
    }

    #[inline]
    pub fn address(&self) -> BDAddr {
        self.peripheral.address()
    }

    /// Signal strength
    #[inline]
    pub async fn rssi(&self) -> Option<i16> {
        self.peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|props| props.rssi)
    }

    /// Local name of the device
    #[inline]
    pub async fn local_name(&self) -> Option<String> {
        self.peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|props| props.local_name)
    }

    /// Disconnect from the device
    #[inline]
    pub async fn disconnect(&self) -> Result<()> {
        self.peripheral.disconnect().await
    }

    /// Services advertised by the device
    pub async fn services(&self) -> Result<Vec<Service>> {
        if !self.peripheral.is_connected().await? {
            self.peripheral.connect().await?;
        }

        let mut services = self.peripheral.services();
        if services.is_empty() {
            self.peripheral.discover_services().await?;
            services = self.peripheral.services();
        }

        Ok(services
            .into_iter()
            .collect::<Vec<_>>())
    }

    /// Number of services advertised by the device
    pub async fn service_count(&self) -> Result<usize> {
        if !self.peripheral.is_connected().await? {
            self.peripheral.connect().await?;
        }

        let mut services = self.peripheral.services();
        if services.is_empty() {
            self.peripheral.discover_services().await?;
            services = self.peripheral.services();
        }

        Ok(services.len())
    }

    /// Characteristics advertised by the device
    pub async fn characteristics(&self) -> Result<Vec<Characteristic>> {
        if !self.peripheral.is_connected().await? {
            self.peripheral.connect().await?;
        }

        let mut characteristics = self.peripheral.characteristics();
        if characteristics.is_empty() {
            self.peripheral.discover_services().await?;
            characteristics = self.peripheral.characteristics();
        }

        Ok(characteristics
            .into_iter()
            .map(|characteristic| Characteristic {
                peripheral: self.peripheral.clone(),
                characteristic,
            })
            .collect::<Vec<_>>())
    }

    /// Get characteristic by UUID
    pub async fn characteristic(&self, uuid: Uuid) -> Result<Option<Characteristic>> {
        if !self.peripheral.is_connected().await? {
            self.peripheral.connect().await?;
        }

        let mut characteristics = self.peripheral.characteristics();
        if characteristics.is_empty() {
            self.peripheral.discover_services().await?;
            characteristics = self.peripheral.characteristics();
        }
        let characteristic = characteristics
            .into_iter()
            .find(|characteristic| characteristic.uuid == uuid);

        Ok(characteristic.map(|characteristic| Characteristic {
            peripheral: self.peripheral.clone(),
            characteristic,
        }))
    }
}

#[derive(Debug, Clone)]
pub enum DeviceEvent {
    Discovered(Device),
    Connected(Device),
    Disconnected(Device),
    Updated(Device),
}
