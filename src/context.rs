use alloc::sync::Arc;
use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

use crate::motion::{
    CommandProducer, MotionCommand, MotorControllerConfig, StateResponse, COMMAND_QUEUE_SIZE,
};
use crate::storage::{NetworkConfiguration, PinConfiguration, StorageManager};
use heapless::spsc::Queue;

pub type SnapshotRef = Arc<StateResponse>;
pub type SnapshotStore = BlockingMutex<CriticalSectionRawMutex, RefCell<SnapshotRef>>;
pub type MotionCmdStore = Mutex<CriticalSectionRawMutex, CommandProducer>;

/// Cross-task motion command producer (waypoints + SetConfig).
pub type MotionCmdTx = &'static MotionCmdStore;
pub type SnapshotHandle = &'static SnapshotStore;

#[derive(Clone, Debug)]
pub(crate) struct StorageCaches {
    pin: PinConfiguration,
    net: NetworkConfiguration,
    motor: MotorControllerConfig,
}

pub type StorageCacheStore = BlockingMutex<CriticalSectionRawMutex, RefCell<StorageCaches>>;

pub(crate) enum StoragePersist {
    Pin(PinConfiguration),
    Net(NetworkConfiguration),
    Motor(MotorControllerConfig),
}

const STORAGE_CMD_CAP: usize = 8;
type StorageCmdChannel = Channel<CriticalSectionRawMutex, StoragePersist, STORAGE_CMD_CAP>;

/// RAM-cached storage handle; flash I/O runs only in `storage_task`.
#[derive(Clone, Copy)]
pub struct StorageHandle {
    caches: &'static StorageCacheStore,
    cmd: &'static StorageCmdChannel,
}

#[derive(Clone, Copy)]
pub struct AppContext {
    pub storage: StorageHandle,
    /// Read-only telemetry published by the motor loop.
    pub motor_snapshot: SnapshotHandle,
    /// Motion / config command producer consumed solely by the motor task.
    pub motion_cmd: MotionCmdTx,
}

impl AppContext {
    pub fn load_snapshot(&self) -> SnapshotRef {
        // Cheap Arc refcount bump under a brief critical section.
        self.motor_snapshot.lock(|cell| cell.borrow().clone())
    }

    /// Publish telemetry. Unique ownership → in-place `copy_from` under a brief CS.
    /// Shared ownership → allocate the new `Arc` *outside* the critical section, then
    /// swap pointers so network tasks are never stalled on large clones.
    pub fn update_snapshot(&self, src: &StateResponse) {
        let need_fresh = self.motor_snapshot.lock(|cell| {
            let mut snap = cell.borrow_mut();
            if Arc::strong_count(&*snap) == 1 {
                Arc::make_mut(&mut snap).copy_from(src);
                false
            } else {
                true
            }
        });
        if need_fresh {
            let mut fresh = StateResponse::default();
            fresh.copy_from(src);
            let new_arc = Arc::new(fresh);
            self.motor_snapshot.lock(|cell| {
                *cell.borrow_mut() = new_arc;
            });
        }
    }

    pub async fn enqueue_motion(&self, cmd: MotionCommand) -> bool {
        self.motion_cmd.lock().await.enqueue(cmd).is_ok()
    }

    pub async fn try_enqueue_config(
        &self,
        mut config: MotorControllerConfig,
    ) -> core::result::Result<MotorControllerConfig, &'static str> {
        let current_version = self.load_snapshot().config.version;
        if config.version > 0 && config.version < current_version {
            return Err("Stale causal version");
        }
        if config.version == 0 || config.version == current_version {
            config.version = current_version.wrapping_add(1);
        }
        if self
            .enqueue_motion(MotionCommand::SetConfig(config.clone()))
            .await
        {
            return Ok(config);
        }
        for _ in 0..50 {
            Timer::after(Duration::from_millis(5)).await;
            if self
                .enqueue_motion(MotionCommand::SetConfig(config.clone()))
                .await
            {
                return Ok(config);
            }
        }
        Err("Command queue full")
    }
}

impl StorageHandle {
    pub fn pin(&self) -> PinConfiguration {
        self.caches.lock(|c| c.borrow().pin.clone())
    }

    pub fn net(&self) -> NetworkConfiguration {
        self.caches.lock(|c| c.borrow().net.clone())
    }

    pub fn motor_config(&self) -> MotorControllerConfig {
        self.caches.lock(|c| c.borrow().motor.clone())
    }

    pub fn set_pin(&self, config: PinConfiguration) {
        let mut config = config;
        config.normalize_operating_mode();
        self.caches.lock(|c| c.borrow_mut().pin = config.clone());
        let _ = self.cmd.try_send(StoragePersist::Pin(config));
    }

    pub fn set_net(&self, config: NetworkConfiguration) {
        self.caches.lock(|c| {
            let caches = &mut *c.borrow_mut();
            caches.net = config.clone();
        });
        let _ = self.cmd.try_send(StoragePersist::Net(config));
    }

    pub fn set_motor_config(&self, config: MotorControllerConfig) {
        self.caches.lock(|c| c.borrow_mut().motor = config.clone());
        let _ = self.cmd.try_send(StoragePersist::Motor(config));
    }

    pub fn set_ssid(&self, ssid: &str) {
        let mut net = self.net();
        net.ssid = alloc::string::String::from(ssid);
        self.set_net(net);
    }

    pub fn set_password(&self, password: &str) {
        let mut net = self.net();
        net.password = alloc::string::String::from(password);
        self.set_net(net);
    }
}

/// Bootstrap static stores: motion command queue, snapshot, storage caches + persist channel.
pub struct AppContextInit {
    pub ctx: AppContext,
    pub motion_consumer: crate::motion::CommandConsumer,
    pub storage_manager: StorageManager,
    pub storage_cmd: &'static StorageCmdChannel,
}

pub fn init_app_context(flash: esp_hal::peripherals::FLASH<'static>) -> AppContextInit {
    static SNAP: static_cell::StaticCell<SnapshotStore> = static_cell::StaticCell::new();
    static CMD_QUEUE: static_cell::StaticCell<Queue<MotionCommand, COMMAND_QUEUE_SIZE>> =
        static_cell::StaticCell::new();
    static CMD_PRODUCER: static_cell::StaticCell<MotionCmdStore> = static_cell::StaticCell::new();
    static CACHES: static_cell::StaticCell<StorageCacheStore> = static_cell::StaticCell::new();
    static STORAGE_CMD: static_cell::StaticCell<StorageCmdChannel> = static_cell::StaticCell::new();

    let mut storage_manager = StorageManager::new(flash);
    let pin = storage_manager
        .get_pin_configuration()
        .unwrap_or_default();
    let net = storage_manager
        .get_network_configuration()
        .unwrap_or_default();
    let motor = storage_manager
        .get_motor_config()
        .unwrap_or_else(|_| MotorControllerConfig::default());

    let motor_snapshot = SNAP.init(BlockingMutex::new(RefCell::new(Arc::new(
        StateResponse::default(),
    ))));

    let queue = CMD_QUEUE.init(Queue::new());
    let (producer, motion_consumer) = queue.split();
    let motion_cmd = CMD_PRODUCER.init(Mutex::new(producer));

    let caches = CACHES.init(BlockingMutex::new(RefCell::new(StorageCaches {
        pin,
        net,
        motor,
    })));
    let storage_cmd = STORAGE_CMD.init(Channel::new());

    let storage = StorageHandle {
        caches,
        cmd: storage_cmd,
    };

    AppContextInit {
        ctx: AppContext {
            storage,
            motor_snapshot,
            motion_cmd,
        },
        motion_consumer,
        storage_manager,
        storage_cmd,
    }
}

/// Sole owner of `StorageManager`; persists cache updates to NVS.
#[embassy_executor::task]
pub async fn storage_task(
    mut manager: StorageManager,
    cmd: &'static StorageCmdChannel,
) {
    loop {
        match cmd.receive().await {
            StoragePersist::Pin(config) => {
                if let Err(e) = manager.set_pin_configuration(&config) {
                    log::error!("Failed to persist pin config: {}", e);
                }
            }
            StoragePersist::Net(config) => {
                if let Err(e) = manager.set_network_configuration(&config) {
                    log::error!("Failed to persist network config: {}", e);
                }
            }
            StoragePersist::Motor(config) => {
                if let Err(e) = manager.set_motor_config(&config) {
                    log::error!("Failed to persist motor config: {}", e);
                }
            }
        }
    }
}
