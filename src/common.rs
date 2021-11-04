pub mod characteristics {
    use btleplug::api::bleuuid::uuid_from_u16;
    use uuid::Uuid;

    pub const HEART_RATE_MEASUREMENT: Uuid = uuid_from_u16(0x2A37);
    pub const BATTERY_LEVEL: Uuid = uuid_from_u16(0x2A19);
}
