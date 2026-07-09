# 04: WiFi & Networking

## Scope

Replace `EspWifi` / `AsyncWifi` / `EspMdns` with `esp-radio` WiFi driver + `embassy-net` stack + minimal mDNS responder. WiFi shares the radio controller with BLE via `coex`.

## Files to Modify

- New: `src/wifi.rs` — extracted WiFi module
- `src/main.rs` — radio init (see [02-entry-point](02-entry-point.md))

---

## Shared Radio Controller

WiFi and BLE share a single `esp_radio::init()` call from `main()`. Both subsystems receive `&'static esp_radio::Controller<'static>`:

```rust
// main.rs (once):
let radio = RADIO.init(esp_radio::init().expect("radio init failed"));

// wifi.rs:
pub async fn start_wifi(
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI,
    app_context: AppContext,
    spawner: &embassy_executor::Spawner,
) -> &'static Stack<'static> {
    // ...
    let mut controller = WifiController::new(radio, wifi_peripheral, Default::default())
        .expect("WifiController::new failed");
    // ...
}
```

Requires `coex` feature on `esp-radio` in `Cargo.toml`.

---

## Target: `src/wifi.rs`

```rust
use embassy_net::{Config as NetConfig, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{WifiController, WifiEvent, Interface};
use static_cell::StaticCell;

use crate::context::AppContext;

const STACK_RESOURCES_COUNT: usize = 5;

pub async fn start_wifi(
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI,
    app_context: AppContext,
    spawner: &embassy_executor::Spawner,
) -> &'static Stack<'static> {
    let (net_conf, ssid, password, hostname) = {
        let sm = app_context.storage.lock().await;
        (
            sm.get_network_configuration().unwrap_or_default(),
            sm.get_ssid().unwrap_or_default(),
            sm.get_password().unwrap_or_default(),
            sm.get_network_configuration()
                .map(|c| c.hostname)
                .unwrap_or_else(|_| alloc::string::String::from("ossm")),
        )
    };

    let mut controller =
        WifiController::new(radio, wifi_peripheral, Default::default()).unwrap();

    let sta_device = Interface::station();

    let net_config = build_net_config(&net_conf);

    let seed = embassy_time::Instant::now().as_ticks();
    let stack = &*{
        static RESOURCES: StaticCell<StackResources<STACK_RESOURCES_COUNT>> = StaticCell::new();
        static STACK: StaticCell<Stack<'static>> = StaticCell::new();
        let resources = RESOURCES.init(StackResources::new());
        STACK.init(Stack::new(sta_device, net_config, resources, seed))
    };

    spawner.must_spawn(net_runner_task(stack));
    spawner.must_spawn(wifi_connection_task(controller, ssid, password));

    // Spawn mDNS responder
    let hostname_static: &'static str = {
        // Leak hostname to 'static for the task lifetime
        let boxed = alloc::boxed::Box::leak(hostname.into_boxed_str());
        &*boxed
    };
    spawner.must_spawn(mdns_responder_task(stack, hostname_static));

    // Wait for IP and log
    loop {
        if let Some(config) = stack.config_v4() {
            log::info!(
                "WiFi connected. IP: {}, mDNS: http://{}.local",
                config.address.address(),
                hostname_static,
            );
            break;
        }
        Timer::after(Duration::from_millis(100)).await;
    }

    stack
}

fn build_net_config(net_conf: &NetworkConfiguration) -> NetConfig {
    if net_conf.dhcp_enabled {
        NetConfig::dhcpv4(Default::default())
    } else {
        use embassy_net::{Ipv4Address, Ipv4Cidr, StaticConfigV4};
        let addr = Ipv4Address::from_bytes(
            net_conf.static_ip.parse().unwrap_or([192, 168, 1, 100])
        );
        let prefix_len = 24; // default /24 if subnet mask parsing simplified
        let gateway = Ipv4Address::from_bytes(
            net_conf.static_gateway.parse().unwrap_or([192, 168, 1, 1])
        );
        let dns = Ipv4Address::from_bytes(
            net_conf.static_dns.parse().unwrap_or([8, 8, 8, 8])
        );

        NetConfig::ipv4_static(StaticConfigV4 {
            address: Ipv4Cidr::new(addr, prefix_len),
            gateway: Some(gateway),
            dns_servers: alloc::vec![dns],
        })
    }
}

#[embassy_executor::task]
async fn net_runner_task(stack: &'static Stack<'static>) {
    stack.run().await;
}

#[embassy_executor::task]
async fn wifi_connection_task(
    mut controller: WifiController<'static>,
    ssid: alloc::string::String,
    password: alloc::string::String,
) {
    loop {
        if ssid.is_empty() {
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }
        // ... connect / reconnect loop (same as before) ...
    }
}
```

---

## mDNS Replacement

Current firmware registers hostname, instance name `"OSSM Sex Machine"`, and `_http._tcp` service on port 80. The minimal responder must answer:

1. **A record** queries for `<hostname>.local`
2. **PTR/SRV** queries for `_http._tcp.local` (service discovery)
3. **A record** in responses pointing to the device's current IP

```rust
#[embassy_executor::task]
async fn mdns_responder_task(
    stack: &'static Stack<'static>,
    hostname: &'static str,
) {
    use embassy_net::udp::{UdpSocket, PacketMetadata};
    use embassy_net::IpAddress;

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buffer = [0u8; 1024];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buffer = [0u8; 1024];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    if socket.bind(5353).is_err() {
        log::error!("Failed to bind mDNS UDP socket on port 5353");
        return;
    }

    let mut packet_buf = [0u8; 512];
    loop {
        if let Ok((n, endpoint)) = socket.recv_from(&mut packet_buf).await {
            if let Some(my_ip) = stack.config_v4().map(|c| c.address.address()) {
                // Check packet_buf[..n] for query matching <hostname>.local or _http._tcp.local
                // Build DNS packet response with A record (my_ip) and SRV port 80
                // socket.send_to(&resp, endpoint).await;
            }
        }
    }
}
```

Alternatively, evaluate `edge-mdns` if it supports `embassy-net` sockets. For MVP, implement A + PTR/SRV for `_http._tcp` to match current `EspMdns::add_service(None, "_http", "_tcp", 80, ...)` behavior.

---

## PHY Calibration

esp-radio may read/write PHY calibration data in the `phy_init` partition at offset `0xF000`. The default espflash partition table includes this partition. No explicit code is needed if using the default partition table, but be aware that erasing flash without preserving `phy_init` can cause WiFi connect issues until recalibrated.

---

## Device Restart

```rust
esp_hal::reset::software_reset();
```
