use crate::{Characteristic, Service};
use btleplug::{
    api::{
        bleuuid::{uuid_from_u16, uuid_from_u32},
        BDAddr, Peripheral as _,
    },
    platform::{Adapter, Peripheral},
    Result,
};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
pub struct Device {
    _adapter: Adapter,
    peripheral: Peripheral,
}

impl Device {
    pub(crate) fn new(adapter: Adapter, peripheral: Peripheral) -> Self {
        Self {
            _adapter: adapter,
            peripheral,
        }
    }

    pub fn address(&self) -> BDAddr {
        self.peripheral.address()
    }

    /// Signal strength
    pub async fn rssi(&self) -> Option<i16> {
        self.peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|props| props.rssi)
    }

    /// Local name of the device
    pub async fn local_name(&self) -> Option<String> {
        self.peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|props| props.local_name)
    }

    /// Disconnect from the device
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
            .map(|service| Service {
                peripheral: self.peripheral.clone(),
                service,
            })
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
    pub async fn characteristic<T: Into<BleUuid>>(
        &self,
        uuid: T,
    ) -> Result<Option<Characteristic>> {
        if !self.peripheral.is_connected().await? {
            self.peripheral.connect().await?;
        }

        let uuid: Uuid = *uuid.into();

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

pub struct BleUuid(Uuid);

impl Deref for BleUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Uuid> for BleUuid {
    fn from(value: Uuid) -> Self {
        BleUuid(value)
    }
}

impl From<u16> for BleUuid {
    fn from(value: u16) -> Self {
        BleUuid(uuid_from_u16(value))
    }
}

impl From<u32> for BleUuid {
    fn from(value: u32) -> Self {
        BleUuid(uuid_from_u32(value))
    }
}
