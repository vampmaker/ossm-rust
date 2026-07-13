use alloc::string::String;

use embassy_executor::Spawner;
use embassy_net::{Config as NetConfig, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Instant, Timer};
use esp_radio::wifi::{Config, Interface, WifiController, sta::StationConfig};
use static_cell::StaticCell;

use crate::context::AppContext;
use crate::storage::NetworkConfiguration;

const STACK_RESOURCES_COUNT: usize = 10;

static STACK: StaticCell<Stack<'static>> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<STACK_RESOURCES_COUNT>> = StaticCell::new();

pub async fn start_wifi(
    wifi_peripheral: esp_hal::peripherals::WIFI<'static>,
    app_context: AppContext,
    spawner: &Spawner,
) -> &'static Stack<'static> {
    let (net_conf, ssid, password, hostname) = {
        let mut sm = app_context.storage.lock().await;
        (
            sm.get_network_configuration().unwrap_or_default(),
            sm.get_ssid().unwrap_or_default(),
            sm.get_password().unwrap_or_default(),
            sm.get_network_configuration()
                .map(|c| c.hostname)
                .unwrap_or_else(|_| String::from("ossm")),
        )
    };

    let controller = WifiController::new(wifi_peripheral, Default::default())
        .expect("WifiController::new failed");

    let sta_device = Interface::station();
    let net_config = build_net_config(&net_conf);
    let seed = Instant::now().as_ticks();

    let resources = STACK_RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(sta_device, net_config, resources, seed);
    let stack = STACK.init(stack);

    spawner.spawn(net_runner_task(runner).unwrap());
    spawner.spawn(wifi_connection_task(controller, ssid, password, hostname).unwrap());

    loop {
        if let Some(cfg) = stack.config_v4() {
            log::info!("Got IP: {}", cfg.address.address());
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
        let addr = parse_ipv4(&net_conf.static_ip, [192, 168, 1, 100]);
        let gateway = parse_ipv4(&net_conf.static_gateway, [192, 168, 1, 1]);
        let dns = parse_ipv4(&net_conf.static_dns, [8, 8, 8, 8]);
        let prefix_len = parse_prefix_len(&net_conf.static_mask);

        NetConfig::ipv4_static({
            let mut static_config = StaticConfigV4 {
                address: Ipv4Cidr::new(addr, prefix_len),
                gateway: Some(gateway),
                dns_servers: Default::default(),
            };
            let _ = static_config.dns_servers.push(dns);
            static_config
        })
    }
}

fn parse_ipv4(value: &str, default: [u8; 4]) -> Ipv4Address {
    let mut octets = default;
    for (i, part) in value.split('.').take(4).enumerate() {
        if let Ok(v) = part.parse::<u8>() {
            octets[i] = v;
        }
    }
    Ipv4Address::from_octets(octets)
}

fn parse_prefix_len(mask: &str) -> u8 {
    if mask.contains('.') {
        let mut bits = 0u8;
        for part in mask.split('.') {
            if let Ok(v) = part.parse::<u8>() {
                bits = bits.saturating_add(v.count_ones() as u8);
            }
        }
        if bits > 0 {
            return bits;
        }
    } else if let Ok(v) = mask.parse::<u8>() {
        return v;
    }
    24
}

#[embassy_executor::task]
async fn net_runner_task(mut runner: Runner<'static, Interface>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn wifi_connection_task(
    mut controller: WifiController<'static>,
    ssid: String,
    password: String,
    hostname: String,
) {
    loop {
        if ssid.is_empty() {
            log::info!("WiFi SSID not set. Configure via USB CLI.");
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        let station_config = Config::Station(
            StationConfig::default()
                .with_ssid(ssid.as_str())
                .with_password(password.clone()),
        );

        if controller.set_config(&station_config).is_err() {
            log::error!("Failed to set WiFi config");
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        match controller.connect_async().await {
            Ok(info) => {
                log::info!(
                    "WiFi connected to {} (ch {}). http://{}.local",
                    ssid,
                    info.channel,
                    hostname
                );
            }
            Err(e) => {
                log::error!("WiFi connect failed: {:?}", e);
            }
        }

        let _ = controller.wait_for_disconnect_async().await;
        log::warn!("WiFi disconnected, retrying...");
        Timer::after(Duration::from_secs(2)).await;
    }
}
