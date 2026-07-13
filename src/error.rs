use core::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum FirmwareError {
    Uart(&'static str),
    Modbus(&'static str),
    Storage(&'static str),
    Wifi(&'static str),
    Config(&'static str),
    Json,
    PinUnavailable,
    MotorNotInitialized,
    QueueFull,
    Ble(&'static str),
    Http(&'static str),
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uart(msg) => write!(f, "UART: {}", msg),
            Self::Modbus(msg) => write!(f, "Modbus: {}", msg),
            Self::Storage(msg) => write!(f, "Storage: {}", msg),
            Self::Wifi(msg) => write!(f, "WiFi: {}", msg),
            Self::Config(msg) => write!(f, "Config: {}", msg),
            Self::Json => write!(f, "JSON parse error"),
            Self::PinUnavailable => write!(f, "GPIO pin unavailable"),
            Self::MotorNotInitialized => write!(f, "Motor controller not initialized"),
            Self::QueueFull => write!(f, "Command queue full"),
            Self::Ble(msg) => write!(f, "BLE: {}", msg),
            Self::Http(msg) => write!(f, "HTTP: {}", msg),
        }
    }
}

pub type Result<T> = core::result::Result<T, FirmwareError>;
