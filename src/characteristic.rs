use btleplug::api::{Characteristic as BtleCharacteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use btleplug::Result;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use uuid::Uuid;

#[derive(Clone)]
pub struct Characteristic {
    pub(crate) peripheral: Peripheral,
    pub(crate) characteristic: BtleCharacteristic,
}

impl Characteristic {
    pub async fn read(&self) -> Result<Vec<u8>> {
        self.peripheral.read(&self.characteristic).await
    }

    pub async fn write_request(&self, data: &[u8]) -> Result<()> {
        self.write(data, WriteType::WithResponse).await
    }

    pub async fn write_command(&self, data: &[u8]) -> Result<()> {
        self.write(data, WriteType::WithoutResponse).await
    }

    async fn write(&self, data: &[u8], write_type: WriteType) -> Result<()> {
        self.peripheral
            .write(&self.characteristic, data, write_type)
            .await
    }

    pub async fn subscribe(&self) -> Result<Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>> {
        self.peripheral.subscribe(&self.characteristic).await?;

        let stream = self.peripheral.notifications().await?;
        let uuid = self.characteristic.uuid;

        Ok(Box::pin(stream.filter_map(move |n| async move {
            if n.uuid == uuid {
                Some(n.value)
            } else {
                None
            }
        })))
    }

    pub async fn unsubscribe(&self) -> Result<()> {
        self.peripheral.unsubscribe(&self.characteristic).await
    }

    pub fn uuid(&self) -> Uuid {
        self.characteristic.uuid
    }
}
