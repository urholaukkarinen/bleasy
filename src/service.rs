use crate::Characteristic;
use btleplug::api::Service as BtleService;
use btleplug::platform::Peripheral;
use uuid::Uuid;

pub struct Service {
    pub(crate) peripheral: Peripheral,
    pub(crate) service: BtleService,
}

impl Service {
    pub fn characteristics(&self) -> Vec<Characteristic> {
        self.service
            .characteristics
            .iter()
            .map(|characteristic| Characteristic {
                peripheral: self.peripheral.clone(),
                characteristic: characteristic.clone(),
            })
            .collect::<Vec<_>>()
    }

    pub fn uuid(&self) -> Uuid {
        self.service.uuid
    }
}
