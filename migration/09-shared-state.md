# 09: Shared State & Concurrency

## Scope

Replace `std::sync::{Arc, Mutex}` with `embassy_sync` primitives and `&'static` references. Redesign `AppContext` for the no_std Embassy world. Migrate the motion command queue.

## Files to Modify

- `src/context.rs` (complete rewrite)
- `src/motion.rs` (`command_producer` Arc removal)
- All files that use `AppContext` (mechanical locking changes)

---

## Target: `AppContext` (context.rs)

```rust
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use crate::motion::MotorController;
use crate::storage::StorageManager;

type CsMutex<T> = Mutex<CriticalSectionRawMutex, T>;

#[derive(Clone, Copy)]
pub struct AppContext {
    pub storage: &'static CsMutex<StorageManager>,
    pub motor_controller: &'static CsMutex<Option<MotorController>>,
}
```

### Key changes:
1. **`Arc` → `&'static`**: `StaticCell` allocation in `main.rs`
2. **`std::sync::Mutex` → `embassy_sync::mutex::Mutex`**: async `.lock().await`
3. **`Box<T>` removed**: `MotorController` stored directly in mutex
4. **`all_pins` removed**: GPIO handled by `GpioPins` at motor init (see [03-motor-uart](03-motor-uart.md))
5. **`Clone` → `Copy`**: two `&'static` references

---

## Eliminating Motor Task Interruptions: Shared Data Ownership & Access Matrix

In ESP-IDF FreeRTOS, WiFi radio ISRs run at high OS priorities, and HTTP/WebSocket tasks holding `Arc<Mutex<MotorController>>` during JSON serialization or socket I/O block the motor loop.

In the `esp-hal` architecture, data ownership is strictly partitioned so that **HTTP requests, WebSocket communication, and WiFi packet processing never delay or interrupt `motor_task`**:

| Shared Data | Owner | Readers & Execution Context | Writers & Execution Context | Lock Duration & Contention Guarantees |
|---|---|---|---|---|
| **Modbus UART Driver (`Modbus57AIM30Motor`)** | Owned exclusively by `motor_task` local stack | `motor_task` (`InterruptExecutor` / Core 1) | `motor_task` (`InterruptExecutor` / Core 1) | **Zero sharing**. No mutex. Network tasks never touch Modbus hardware or RS-485 pins. |
| **`MotorController` (trajectory math & state)** | `AppContext.motor_controller` (`&'static CsMutex<Option<MotorController>>`) | • `motor_task`: reads config during `compute_cycle`<br>• HTTP / WS / BLE: snapshot reads via `get_current_state`<br>• `nvs_saver_task`: checks dirty version | • `motor_task`: updates `t`, position, and loop stats in `compute_cycle`<br>• HTTP / WS / BLE / CLI: writes new config via `set-config` | **Sub-microsecond lock (< 2 µs)**. Mutex is held *only* for pure CPU float math or memory cloning. **All JSON serialization (`serde_json`), network socket I/O, and UART writes occur outside the lock.** |
| **Motion Command Queue (`CMD_QUEUE`)** | Static heapless SPSC (`StaticCell<Queue<MotionCommand, 64>>`) | `motor_task` owns SPSC `Consumer` | HTTP / WS / BLE / CLI share SPSC `Producer` via `CsMutex` | **Lock-free dequeue** by `motor_task`. Enqueuing takes < 1 µs without blocking consumer. |
| **`StorageManager` (NVS Settings)** | `AppContext.storage` (`&'static CsMutex<StorageManager>`) | • Boot (`main.rs`)<br>• HTTP / BLE / CLI config queries | • `nvs_saver_task`<br>• HTTP / BLE / CLI config updates | Isolated in a separate mutex. NVS flash erase/write operations never block `MotorController` or `motor_task`. |

### Execution Priority & Core Allocation Guarantees

1. **ESP32-C6 (Single-Core RISC-V)**:
   - `motor_task` runs on `InterruptExecutor` (`software_interrupt1` at **Priority 3**, above Thread level).
   - WiFi connection, `embassy-net` TCP/IP stack, `picoserve` HTTP/WebSocket servers, and NVS tasks run on the Thread Executor (**Priority 0**).
   - When the motor step interrupt fires, the hardware CPU interrupt preempts any ongoing HTTP request parsing, WebSocket framing, or WiFi background task instantly.
2. **ESP32-S3 (Dual-Core Xtensa)**:
   - **Core 0**: Runs WiFi radio ISRs, HTTP/WebSocket servers, BLE GATT, and NVS saver.
   - **Core 1**: Dedicated exclusively to `motor_task` (`run_motor`).
   - Zero physical CPU contention between network communication and motor steps.

---

## Motor Split Architecture

`AppContext` holds only `MotorController` (state/math), **not** the motor driver:

```
AppContext.motor_controller  →  MotorController (config, compute_cycle, command queue)
run_motor() local variable   →  Modbus57AIM30Motor (UART I/O: homing, write_position, cycle)
```

HTTP/BLE/CLI access config and enqueue waypoints via the mutex. UART I/O happens only in the motor task on the interrupt executor.

---

## Motion Command Queue

Current `motion.rs` uses:

```rust
command_producer: Arc<Mutex<CommandProducer>>,
// ...
pub fn get_command_sender(&self) -> Arc<Mutex<CommandProducer>> { ... }
```

HTTP, BLE, and CLI enqueue `MotionCommand` (waypoints, reset-timestamp) through this. **This must be preserved.**

### Target: replace Arc with `&'static`

```rust
use heapless::spsc::{Queue, Producer, Consumer};
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

const COMMAND_QUEUE_SIZE: usize = 64;

// Allocated once in main.rs or MotorController::new():
static CMD_QUEUE: StaticCell<Queue<MotionCommand, COMMAND_QUEUE_SIZE>> = StaticCell::new();
static CMD_PRODUCER: StaticCell<Mutex<CriticalSectionRawMutex, Producer<'static, MotionCommand, COMMAND_QUEUE_SIZE>>> =
    StaticCell::new();

// In MotorController::new():
let queue = CMD_QUEUE.init(Queue::new());
let (producer, consumer) = queue.split();
let producer_ref = CMD_PRODUCER.init(Mutex::new(producer));

// In MotorController:
command_producer: &'static Mutex<CriticalSectionRawMutex, Producer<'static, MotionCommand, COMMAND_QUEUE_SIZE>>,

pub fn get_command_sender(&self) -> &'static Mutex<CriticalSectionRawMutex, Producer<'static, MotionCommand, COMMAND_QUEUE_SIZE>> {
    self.command_producer
}
```

### Enqueue from HTTP (async):

```rust
let sender = mc.get_command_sender();
sender.lock().await.enqueue(MotionCommand::AppendWaypoints(wps))
    .map_err(|_| FirmwareError::QueueFull)?;
```

### Enqueue from CLI (sync callback):

Use `embassy_sync::blocking_mutex::Mutex` for the producer if async lock is unavailable from sync context, or use `critical_section::with` + a `try_enqueue` on the heapless SPSC queue directly (SPSC producer is `Sync` if only one producer task exists — but HTTP and BLE also produce, so the mutex is required).

**Recommended**: Wrap the producer in both an async mutex (for HTTP/BLE) and expose a `try_enqueue_command()` helper that uses `critical_section::with` for CLI.

---

## Locking Pattern Changes

### Async context (HTTP, BLE, nvs_saver)

```rust
let config = {
    let mc_opt = app_context.motor_controller.lock().await;
    mc_opt.as_ref().map(|mc| mc.get_config())
};
```

### Motor task (interrupt executor, hot loop)

```rust
let target = {
    let mut mc_lock = app_context.motor_controller.lock().await;
    mc_lock.as_mut().map(|mc| mc.compute_cycle())
};
// I/O outside lock:
motor.write_position(position, speed).await?;
motor.cycle().await?;
```

Lock held for <5 µs (pure math, no I/O, no await inside lock).

### CLI (sync callback)

Use blocking mutex for storage access (see [08-serial-cli](08-serial-cli.md)).

---

## Cross-Core Constraints (ESP32-S3)

On ESP32-S3, the motor task runs on Core 1. `AppContext` (`&'static` references) is `Copy` and safe to share across cores. `CriticalSectionRawMutex` uses a global critical section for cross-core exclusion — it does **not** disable interrupts on both cores individually, but prevents concurrent access.

UART async driver must be initialized on Core 1 (see [02-entry-point](02-entry-point.md)).

---

## Cross-Executor Communication

The motor task runs on the `InterruptExecutor`; HTTP/BLE run on the thread executor. They communicate only through:

1. `app_context.motor_controller` mutex (config reads/writes, `compute_cycle`)
2. `command_producer` queue (waypoint streaming)

No additional channels needed if the above two paths are preserved.

---

## Migration Checklist

| Pattern | Current | Target |
|---|---|---|
| Lock | `.lock().unwrap()` | `.lock().await` |
| Pass context | `app_context.clone()` | `app_context` (Copy) |
| Type | `Arc<Mutex<Box<T>>>` | `&'static CsMutex<T>` |
| Field | `app_context.storage_manager` | `app_context.storage` |
| Field | `app_context.all_pins` | Removed (GpioPins) |
| Command queue | `Arc<Mutex<Producer>>` | `&'static Mutex<Producer>` |

Files affected: `main.rs`, `http_api.rs`, `rpc.rs`, `ble_api.rs`, `command.rs`, `motion.rs`
