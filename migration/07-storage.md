# 07: Persistent Storage (NVS)

## Scope

Replace `esp_idf_svc::nvs::EspNvs` with `esp-nvs` + `esp-storage` for bare-metal NVS access. Preserve backward compatibility with devices already flashed with ESP-IDF NVS data.

**Phase note**: Storage is initialized in Phase 1 (before WiFi/motor), because all subsystems read NVS at boot.

## Files to Modify

- `src/storage.rs` (internal implementation changes; public API preserved)

---

## Target: `StorageManager` using `esp-nvs`

```rust
use esp_nvs::Nvs;
use esp_storage::FlashStorage;
use alloc::string::String;
use serde::{Serialize, de::DeserializeOwned};

use crate::error::{FirmwareError, Result};
use crate::motion::MotorControllerConfig;

// Default espflash partition table (see espflash save-image --merge output)
const NVS_PARTITION_OFFSET: u32 = 0x9000;
const NVS_PARTITION_SIZE: u32 = 0x6000;
const NVS_NAMESPACE: &str = "ossm";

pub struct StorageManager {
    nvs: Nvs<FlashStorage<'static>>,
}

impl StorageManager {
    pub fn new(flash: esp_hal::peripherals::FLASH) -> Self {
        let storage = FlashStorage::new(flash);
        let nvs = Nvs::new(NVS_PARTITION_OFFSET, NVS_PARTITION_SIZE, storage)
            .expect("Failed to initialize NVS");
        Self { nvs }
    }

    // ... get_string, set_string, get_json, set_json (unchanged API) ...

    pub fn get_network_configuration(&self) -> Result<NetworkConfiguration> {
        self.get_json("net_conf")
            .or_else(|_| Ok(NetworkConfiguration::default()))
    }
}
```

---

## NVS Partition Table

```
# Name,   Type, SubType, Offset,  Size,   Flags
nvs,      data, nvs,     0x9000,  0x6000,
phy_init, data, phy,     0xf000,  0x1000,
factory,  app,  factory, 0x10000, 0x300000,
```

The `esp-nvs` crate reads the raw NVS pages at the specified offset, parsing the same on-flash format that ESP-IDF writes. Existing devices flashed with ESP-IDF firmware will have WiFi credentials, pin config, motor config, and network config preserved.

---

## PHY Init Partition

The `phy_init` partition at `0xF000` stores WiFi/BLE RF calibration data. esp-radio uses this automatically when present. Do not erase it during firmware updates. If missing, esp-radio recalibrates on first boot (slower connect).

---

## Thread Safety

`StorageManager` is behind `&'static embassy_sync::mutex::Mutex<CriticalSectionRawMutex, StorageManager>`.

- Flash writes must not happen from interrupt context.
- `nvs_saver_task` is the primary writer for motor config.
- CLI may write directly (WiFi credentials, pin config) — mutex serializes access.
- On ESP32-S3, `CriticalSectionRawMutex` provides cross-core exclusion via a global critical section (not per-core interrupt masking).

---

## nvs_saver_task Logic

The current firmware has a bug: `last_saved_version != 0` skips the first config change after boot. The migrated version saves whenever version changes:

```rust
if ver != last_saved_version {
    last_saved_version = ver;
    Some(mc.get_config())
} else {
    None
}
```

See corrected task in [02-entry-point](02-entry-point.md).

---

## API Changes Summary

| Aspect | Current | Target |
|---|---|---|
| Constructor | `StorageManager::new(EspDefaultNvsPartition)` | `StorageManager::new(peripherals.FLASH)` |
| Internal storage | `EspNvs<NvsDefault>` | `esp_nvs::Nvs<FlashStorage>` |
| Namespace | `EspNvs::new(partition, "ossm", true)` | Passed to each `get_str`/`set_str` call |
| Error type | `anyhow::Result<T>` | `crate::error::Result<T>` |
| Allocation | `vec![0u8; 1024]` (heap) | `[0u8; 1024]` (stack) |
