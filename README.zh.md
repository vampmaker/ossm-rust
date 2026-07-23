# OSSM-Rust 固件

本仓库为用 Rust 编写的开源通用性控制嵌入式软件（OSSM Controller Firmware）。本固件专为 ESP32-C6 或 ESP32-S3 微控制器量身打造，通过 RS-485 Modbus 协议驱动 57AIM30 一体化伺服电机（集成驱动）。

本指南专为所有用户编写，即使您没有任何编程、硬件或工程技术背景，也能轻松上手！请仔细按照以下图文步骤进行接线与配置，即可顺利运行您的设备。

## 第一部分：所需硬件

以下是搭建 OSSM 控制器所需的硬件组件清单。

| 组件 | 规格描述 | 选型建议与备注 |
| --- | --- | --- |
| **微控制器** | ESP32-C6 或 ESP32-S3 开发板 | 任何带有 Type-C (USB-C) 接口的标准 ESP32-C6 或 ESP32-S3 开发板均可使用。<br><br>**关于 USB 接口的重要提示：** 很多市面上的开发板配有**两个** Type-C 接口（通常分别标注为 "USB" / "Native" / "JTAG" 与 "UART" / "COM"）。请务必将数据线插在 **Native JTAG / USB 接口**（即原生 USB 接口），而非 USB 转串口（UART）接口上！原生 JTAG 接口直接连接主控芯片，通信传输更快，且在网页中能即插即用，无需额外安装复杂的电脑驱动程序。 |
| **驱动电机** | 57AIM30 一体化伺服电机（内置驱动） | **重要提示：** 请务必选购 `57AIM30` 型号，而非 `57AIM30H`。`57AIM30` 额定转速为 1500 RPM，额定扭矩为 0.96 N·m，其性能曲线最适合本项目的负载需求。 |
| **总线收发器** | MAX3485 (RS-485) 转换模块 | 用于实现 ESP32 微控制器与伺服电机之间的 RS-485 Modbus 串行总线通信。 |
| **主电源（电机）** | 24V 直流电源适配器 | 为一体化伺服电机供电。请确保输出功率与电流能够满足您的负载需求。 |
| **逻辑电源（ESP32）**| 5V USB 充电器 | 任何标准的智能手机 5V 充电头搭配 USB-C 数据线即可，在系统完成配置后为开发板稳定供电。 |
| **线缆与接插件** | - DC 5.5×2.1/2.5mm 母头插座<br>- **KF2EDGK-3.81 6P** 绿色端子插头<br>- **PHB2.0 2×5P** 通信插头/连接线<br>- 杜邦线 | **关于电机接座端子的重要提示：** 请检查购买电机时包装盒内的配件。如果电机未附带配套的绿色与白色插头，您需要自行购买：<br>• **电机前侧 6 针动力电源座：** 需要购买 **KF2EDGK-3.81 6P** 间距3.81mm绿色拔插式接线端子。<br>• **电机后侧 10 针通信与逻辑电源座：** 需要购买 **PHB2.0 2×5P** (2.0mm间距，2行5针) 插头或预压制好的排线。<br>DC 母头插座用于将 24V 电源适配器引出，并接入到绿色的 KF2EDGK 动力接线端子中。 |

## 第二部分：硬件接线与组装

正确的接线是设备正常稳定运行的关键。如果您是第一次接触硬件，不用担心，只需按照步骤一步一步对照即可！在给任何设备接通电源之前，请务必仔细核对每一个线路连接。

### 硬件接线系统总览图
下图为整体硬件系统接线示意图，清晰展示了电源适配器、ESP32 开发板、MAX3485 通信收发模块以及 57AIM30 一体化伺服电机前、后插座端子之间的完整连线方式：

<div align="center">
  <img src="./assets/wiring_diagram_zh.svg" alt="OSSM 硬件接线总览图" width="100%"/>
</div>

> [!TIP]
> **插头该接哪个插座？**
> 您的 57AIM30 电机上有两个插座接口：
> • **前侧 6 针动力插座：** 使用绿色 **KF2EDGK-3.81 6P** 插头，用于输入 24V 大电流动力电源。
> • **后侧 10 针控制插座：** 使用白色 **PHB2.0 2×5P** 插头，用于提供 5V 控制芯片工作电源及 RS-485 通信信号线。

### 第一步：电机主电源接线（前侧 6 针动力端子）

该端子负责为伺服驱动及电机定子提供 24V 高压动力。请在此处使用绿色的 **KF2EDGK-3.81 6P** 插头：

| 电机引脚 (+V) | --> | 24V 直流电源适配器（正极 `+`） |
| --- | --- | --- |
| 电机引脚 (GND) | --> | 24V 直流电源适配器（负极 `-`） |

> [!CAUTION]
> **注意极性！** 在将 24V 电源适配器插入插座通电之前，请务必再三核对正极 (`+`) 与负极 (`-`) 绝对不能接反！

### 第二步：通信总线与逻辑电源接线（后侧 2×5 针控制端子与 ESP32）

电机背部的控制端子使用白色的 **PHB2.0 2×5P** 插头。它不仅为电机内部的智能芯片提供 5V 逻辑电源，还通过 RS-485 收发模块 (MAX3485) 与 ESP32 进行控制指令的双向通信。

首先，将电机的 RS-485 通信数据线连接至 MAX3485 模块：

| 电机引脚 (485A) | --> | MAX3485 模块 (A 端口) |
| --- | --- | --- |
| 电机引脚 (485B) | --> | MAX3485 模块 (B 端口) |

随后，将您的 ESP32 开发板与 MAX3485 模块以及电机后侧控制端子进行连接：

| ESP32 开发板引脚 | --> | 目标硬件引脚 | 功能作用说明 |
| --- | --- | --- | --- |
| 5V (或 VBUS / VIN) | --> | 电机引脚 (5V) | 为电机内部控制逻辑芯片供电 |
| GND (地线) | --> | 电机引脚 (COM / GND) | 共地，确保信号稳定与电气回路安全 |
| 3.3V (或 3V3) | --> | MAX3485 模块电源 (VCC / 3.3V) | 为 RS-485 数据通信模块供电 |
| GND (地线) | --> | MAX3485 模块地 (GND) | RS-485 模块共地 |
| GPIO 18 (TX) | --> | MAX3485 数据发送端 (DI / TX) | 向电机发送各种控制指令 |
| GPIO 19 (RX) | --> | MAX3485 数据接收端 (RO / RX) | 接收电机的反馈与遥测数据 |
| GPIO 20 | --> | MAX3485 收发方向控制 (DE / RE) | 控制 RS-485 总线的收发方向转换 |

> [!NOTE]
> **关于 GPIO 引脚分配的说明**：固件 NVS 中默认的 Modbus 引脚为 GPIO 18 / 19 / 20（两款芯片默认值相同）。在 ESP32-S3 上，19 / 20 常被 USB Serial/JTAG 占用——请将 Modbus TX/RX/DE-RE 接到空闲 GPIO，并通过网页界面、串口命令（`set pin.modbus_tx` 等）或烧录器配置面板写入 NVS。固件**不会**自动探测 GPIO 接线；开机时仅在电机未以 `115200` 响应时扫描 Modbus 波特率 / 从站 ID。

### 第三步：为 ESP32 开发板供电

最后，使用 USB-C 数据线将 ESP32 开发板连接至电脑（用于下一步烧录），或连接至标准的 5V USB 充电头（日常独立运行时）：

| ESP32 Type-C 接口 | --> | 5V USB 充电头 / 电脑 USB 接口 |
| --- | --- | --- |

> [!IMPORTANT]
> 再次提醒：如果您的 ESP32 开发板上有两个 Type-C 接口，请务必把线插在 **Native JTAG / USB 接口**（而非 UART / COM 串口转换接口）上！

### 从源码编译（开发者可选）

普通用户完全无需自行搭建开发环境或编译代码——我们在“第三部分”为您准备好了现成的编译版本！如果您是开发者，想要从源代码进行定制与构建，本项目采用 Espressif 官方的 `esp` Rust 工具链（已在 `rust-toolchain.toml` 中精确配置）。通过 `.cargo/config.toml` 中预设的快捷命令即可一键编译：

```
cargo b-c6 --release    # 为 ESP32-C6 平台编译固件
cargo b-s3 --release    # 为 ESP32-S3 平台编译固件
```

构建目标由 Cargo 特性 / 别名（`esp32c6` / `esp32s3`）选择。两款芯片的默认 Modbus GPIO 编号相同；若板级接线不同，请在 NVS 中修改引脚配置。

## 第三部分：烧录固件

您无需从源码编译。预编译好的**合并**固件镜像（`ossm-esp32c6.bin` / `ossm-esp32s3.bin`）在仓库 **Releases** 中提供，并附带基于 [Web Serial API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API) 的独立网页烧录器（`flasher.html`）——无需安装命令行工具。

> [!IMPORTANT]
> **请使用 Native USB / JTAG 接口！**
> 若开发板有**两个** Type-C 口（常见标注 "USB" / "Native" / "JTAG" 与 "UART" / "COM" / "CP2102"），**请始终插入 Native JTAG / USB 口**。该口直连芯片，可在 Chrome / Edge 中即插即用，通常不必安装额外 USB–UART 驱动。

### 烧录器界面一览

`flasher.html` 为单页布局：

| 区域 | 作用 |
| --- | --- |
| **状态栏**（顶部） | 连接状态，以及 **Connect**、**Monitor**、**Stop**、**Reset**、**Disconnect** |
| **Flash Firmware**（左侧） | 拖入 / 选择 `.bin`、可选烧录地址（默认 `0x0`）、**Flash Firmware** |
| **Device Configuration**（右侧） | WiFi、Modbus GPIO、Modbus 时序、BLE，然后 **Send Configuration** |
| **Console**（左下） | 烧录器进度与 CLI ACK 日志 |
| **Serial**（右下） | 设备 UART 实时输出（启动日志、WiFi IP、电机提示） |

表单内容会保存在浏览器本地（`localStorage` 键 `ossm-flasher-device-config-v1`），下次打开可复用。**Reset to defaults** 可将表单恢复为默认引脚 / 时序。

语言：嵌入式控制界面、`flasher.html` 与 `motor-control.html` 均支持 **English** / **中文**（页头切换）。偏好保存在 `localStorage` 键 `ossm_locale`（`en` | `zh`）。未设置时按浏览器语言（`zh*` → 中文，否则英文）。

### 烧录步骤

1.  从 **Releases** 下载并解压最新发布包（`.zip`）。
2.  用 USB-C **数据线**将 ESP32-C6 或 ESP32-S3 接到电脑的 **Native USB / JTAG** 口。
3.  用 **Google Chrome** 或 **Microsoft Edge** 打开 `flasher.html`（需 Web Serial；桌面浏览器；Safari / Firefox 及多数移动浏览器不支持）。可直接双击文件或拖入标签页，无需本地 Web 服务。
4.  点击状态栏 **Connect**，在浏览器弹窗中选择 ESP32 串口。
    > [!TIP]
    > **如何确认是哪一个串口？**
    > 若列表有多个端口，可用插拔法：记下列表 → 取消 → **拔掉** ESP32 → 再点 **Connect** 看哪一项消失 → 插回 → **Connect** 并选择重新出现的端口。
5.  在 **Flash Firmware** 区域拖入（或浏览选择）对应芯片的合并镜像：
    - ESP32-C6 → `ossm-esp32c6.bin`
    - ESP32-S3 → `ossm-esp32s3.bin`
    **Flash address** 保持 `0x0`（除非您明确需要其他偏移；发布包中的合并镜像从 `0x0` 写入）。
6.  点击 **Flash Firmware**。关注进度条与 **Console** 面板（`Flash complete` / 设备复位）。
7.  烧录结束后工具会硬复位芯片，状态变为 **Flash complete**。通常**无需**再点 Connect —— USB 端口仍保持占用，可直接配置。

> [!NOTE]
> **Connect** 会进入 ROM 引导下载模式（烧录所需）。烧录成功后工具会离开 bootloader，以便在同一页面配置并监视正在运行的固件。

## 第四部分：网络与设备配置

烧录完成后请继续留在同一 `flasher.html` 页面。填写右侧 **Device Configuration**，然后点击 **Send Configuration**。

### 配置项

| 分组 | 填写内容 |
| --- | --- |
| **WiFi** | 开关 **Enable WiFi**；开启时填写 SSID 与密码。若仅用 BLE / USB，可关闭 WiFi。 |
| **GPIO Pins** | Modbus **TX** / **RX** / **DE/RE**（默认 `18` / `19` / `20`）。接线不同时请改（尤其 ESP32-S3 上 19/20 可能被 USB 占用）。 |
| **Motor / Modbus** | 可选时序：读超时 (ms)、字节间超时 (µs)、帧间静默 (µs)、扫描延迟 (µs)。填 `0` 使用固件自动默认值（115200 下：**10 ms** 帧超时、**750 µs** 字节间、**350 µs** 帧间）。界面中波特率 `115200` 与设备 ID `1` 为固定显示。 |
| **BLE** | **Enable Bluetooth Low Energy (BLE)** — 默认开启，不需要时可关闭。 |

### 发送配置

1.  保持 USB 连接。状态应为 **Flash complete**、**Connected to …** 或 **Configuration complete**（这些状态才会启用 **Send Configuration**）。若曾点过 **Disconnect**，请先再点 **Connect**。
2.  点击 **Send Configuration**。烧录器会复位设备、等待启动，再依次发送串口 CLI（`set net.*`、`set pin.*`，最后 `reset`），并在 **Console** 中等待每条 ACK。
3.  配置成功后会自动进入 **Serial** 监视。请留意 WiFi 连接成功日志以及类似 `http://<hostname>.local` 或分配到的 IP。**请记下该 IP**（若本机 mDNS 可用也可访问 `http://ossm.local`），随后用浏览器打开控制面板。
4.  **Stop** 可暂停监视，**Monitor** 可再次附着，**Reset** 仅复位芯片而不发送配置，**Disconnect** 释放 Web Serial 端口。

若配置失败（等待 ACK 超时），请查看 **Console** / **Serial**，确认使用的是 Native USB 口，必要时重新 **Connect** 后再试 **Send Configuration**。

**关于电机初始化**：每次开机固件都会经 Modbus 联系伺服。若电机在 `115200` 无响应，会扫描其他波特率 / 从站 ID。若在其他速率找到电机，会将其改写为 `115200`，并在串口日志中提示您对电机 **24V** 电源断电约 3 秒后再上电。

## 第五部分：设备使用

当设备成功连入您的局域网后，您便可通过自带的嵌入式 Web 交互控制面板、自动化控制脚本或丰富的网络 API 对其进行全方位的实时控制；同时，USB 串口命令行依然保留了完整的系统控制与诊断能力。

### 嵌入式 Web 交互界面

固件内部集成并托管了一个精美直观的 Web 操作面板。请确保您的电脑、手机或平板电脑与 ESP32 连入同一个局域网，打开浏览器并在地址栏中输入您在第四部分记下的 ESP32 IP 地址。

访问示例：`http://192.168.1.123`

在该控制面板中，您可以自由调整往复速度（BPM）、行程深度、顶/底锚定方向，利用可视化编辑器绘制复杂的样条曲线波形（Spline Wave），编排时间轴控制宏，以及在线修改软硬件设置并实现系统平滑重启。

核心界面与连接特性包含：
- **实时电机行程图表（可切换并持久化）**：可视化展现全行程区间（`0% – 100%`）、物理极限位置（`pos_min` / `pos_max`）、当前运动区间（左右极限点标记），以及以 30 FPS 实时展示电机运动位置的动画组件。顶部活动图标可一键切换显示/隐藏，并在浏览器中自动记忆切换状态。
- **高韧性实时数据遥测 (`WsDataManager`)**：前端采用单所有者 WebSocket 专职通信管理器 (`client.ts`)，在行程图或设置面板打开时以高达 **30 FPS (`33ms`)** 速率经 WiFi WebSocket（`/ws/command`）实时推送状态数据与底层控制循环统计指标 (`ups`、`dt_min_ms`、`dt_max_ms`、`dt_avg_ms`、`dt_mdev_ms`)。面板关闭时，UI 会回退为 **1 Hz** 的 REST `/state` 轮询以节省 WebSocket 会话槽位。BLE 实时遥测走独立路径（`ble.ts` / GATT `CHAR_STATE` 通知），不经过该 WebSocket 管理器。固件端采用**收发分离异步任务架构** (`edge_nal::TcpSplit`) 结合堆分配字符串与 **4 缓冲区共享缓冲池 (`NET_BUFFER_POOL`)**，即使在高并发 HTTP REST 请求与持续 WebSocket 遥测并存时也不会发生缓冲区阻塞或掉线。
- **因果配置版本号**：每次电机配置变更都会递增单调的 `version` 字段。UI 跟踪 `authoritativeVersion`，并忽略过期的后台快照，避免滑块编辑在并发 `/state` 推送下被回滚。
- **交互式运动控制、样条编辑器与宏播放器**：调整 BPM（速度）、行程深度、顶/底锚定、行程反转、暂停/恢复模式，用交互式样条控制点设计自定义周期轨迹，或编排带导入/导出、间隙定位与循环播放的**控制宏**（波形按钮 **宏**）。**Funscript** 模式同样由客户端按时间轴驱动。

### 串口命令与调试

### 串口命令

也可通过 USB 串口（`115200` 波特，`\r\n`）控制设备。在 `flasher.html` 中请使用 **Serial** 面板（或 **Monitor**）进行交互式 CLI；**Console** 仅显示烧录 / 配置工具自身的日志。同一套命令也适用于任意串口终端。

固件 `console` 任务独占 USB Serial/JTAG 与 UART0。日志输出（`log::*` / `MODBUS_DBG`）与 CLI 回显/应答共用物理链路，但走**独立路径**：优先 **`CLI_OUT_CH`** 队列、CLI 专用 USB TX 环形缓冲、**RX 优先**轮询与分批日志发送，使诊断日志洪泛时主机串口 CLI 仍可响应。非调试模式下电机 `ups` 在 ESP32-C6 上仍约 **320 Hz**；`modbus_debug` 会增加 USB 日志量，但 homing 稳定后 `ups` 通常仅比非调试低几个百分点。

配置采用类似 nmcli 的 **`get`** / **`set`** 点分路径。设备上输入 **`paths`**（或 `help get` / `help set`）查看完整目录。

```
get <path>                     - 获取配置节 JSON 或标量值
set <path> <value>             - 设置配置（含空格的字符串请加引号）
paths                          - 在设备上打印完整路径/取值目录

# 配置节（完整 JSON）
get pin | get net | get motor

# 示例
set net.ssid "MyNetwork"
set net.password secret
set net.wifi_enabled true
set pin.modbus_tx 2
set pin.modbus_debug false
set motor.paused true
set motor {"bpm":36,"depth":1.0,"wave_func":"sine","paused":true}

# 配置路径目录
pin.modbus_tx / modbus_rx / modbus_de_re          GPIO 0..48
pin.modbus_timeout_ms                             0..1000（0 = 默认约 10 ms）
pin.modbus_rx_timeout_us                          0..200000（0 = 自动）
pin.modbus_scan_delay_us                          0..200000（0 = t3.5）
pin.modbus_inter_frame_delay_us                   0..200000（0 = 自动）
pin.ble_enabled / pin.modbus_debug                true|false（debug 需重启）
pin.operating_mode                                servo|rtu_relay（需重启）
net.wifi_enabled / net.dhcp_enabled               true|false
net.ssid / net.password / net.hostname          字符串
net.static_ip / static_mask / static_gateway / static_dns   IPv4
motor.bpm                                         > 0
motor.depth                                       0.01..1
motor.depth_top / reversed / paused / streaming   true|false
motor.wave_func                                   sine|thrust|spline
motor.sharpness                                   0.01..0.99
motor.paused_position                             0..1
motor.spline_points                               空格分隔浮点数
motor（批量）                                      完整 MotorControllerConfig JSON
inject                                            get inject | set inject <off|leading|trailing|both> <nbytes 0..64>

# 动作命令（非配置）
reset                          - 软重启
get-state / get-status         - 遥测 JSON
reset-timestamp                - 重置运动流时间
set-waypoints <json>           - 替换航点缓冲
append-waypoints <json>        - 追加航点
```

设置成功日志格式为 `{path} set to {value}`（如 `pin.modbus_tx set to 2`）。标量读取为 `{path}: {value}`。

### 自动化设备测试（开发者）

`scripts/` 下的实机脚本（使用 `uv run` 执行）；在 `.env` 中配置 `DEVICE_IP`：

| 脚本 | 用途 |
| --- | --- |
| `./scripts/test_console_fairness.py` | `modbus_debug` + `MODBUS_DBG` TX 洪泛下串口 CLI 应答（`POST /modbus-inject`，电机运行） |
| `./scripts/test_modbus_debug_device.py` | 经 **probe-rs RTT** 验证 Modbus CRC 重同步 / 注入（避免 USB ACM 写阻塞） |
| `./scripts/test_modbus_resync.py` | 纯主机 CRC/重同步镜像测试（无需硬件） |
| `./scripts/stress_dt_max.py` | HTTP+WebSocket 负载；断言 `dt_max_ms` < 4.5 ms |

### 多模 CLI 控制工具 (`ossm.py`)

本仓库在 `scripts/` 目录下提供了一个功能强大的统一命令行工具 `ossm.py`。该脚本采用标准的 PEP 723 自包含格式声明，借助 [`uv`](https://docs.astral.sh/uv/) 包管理器，您无需繁琐地创建和配置 Python 虚拟环境即可直接运行。

该工具支持通过三种基础通信模式与 OSSM 硬件交互：**WiFi**（HTTP REST API & WebSocket JSON-RPC 2.0）、**低功耗蓝牙 (BLE)**（`ble`）以及 **USB 串口**（`serial`）。

**环境准备：** 确保您的系统已安装 [uv](https://docs.astral.sh/uv/)（安装命令：`curl -LsSf https://astral.sh/uv/install.sh | sh`）。

**基本语法与常用操作示例：**
```bash
# 查看所有可用命令、模式及参数描述
./scripts/ossm.py --help

# 通过 WiFi 查询设备实时状态与参数（默认采用 WiFi 模式，默认 IP 为 192.168.24.63，也可通过 -i 参数指定）
./scripts/ossm.py status -i 192.168.1.123

# 通过蓝牙 BLE 模式获取状态（系统会自动扫描并连接周边广播的 OSSM 蓝牙设备）
./scripts/ossm.py status -m ble

# 在 WiFi 模式下，设置电机以 40 BPM 速度、80% 行程深度运行样条波形
./scripts/ossm.py run --bpm 40 --depth 0.8 --wave spline -m wifi

# 通过蓝牙 BLE 立即暂停电机，或使其安全停靠在行程最底层（0.0 位置）
./scripts/ossm.py pause --pos 0.0 -m ble
./scripts/ossm.py resume -m ble

# 切换至串口模式，查看或重新配置 RS-485 Modbus 引脚映射及蓝牙使能开关
./scripts/ossm.py pins -m serial -p /dev/ttyACM0

# 在 WiFi 模式下，查看或修改网络参数与 mDNS 主机名
./scripts/ossm.py net -m wifi

# 启动终端交互式面板（TUI Dashboard），实时监控电机坐标与速度数据流
./scripts/ossm.py monitor -m ble

# 写入示例控制宏 JSON，然后播放
./scripts/ossm.py macro --init /tmp/warmup.json
./scripts/ossm.py macro /tmp/warmup.json --loop --speed 1.0 -m wifi
```

当作为 Python 第三方库在代码中导入时，`ossm.py` 还导出了经过 Pydantic v2 严格校验的数据模型（如 `MotorControllerConfig`、`StateResponse`、`PinConfiguration`、`NetworkConfiguration` 等）和 `DeviceBackend` 核心类，极大地简化了开发者编写自动化测试用例或高级集成应用的代码工作。

### 高级控制：样条曲线波形 (Spline Wave)

`spline`（样条曲线）是一种极其强大的高级波形功能，专门用于构建完全自定义的运动轨迹。通过它，您不再被局限于传统的 `sine`（正弦波）或 `thrust`（推力波）等预设模式，而是可以自由定义一组坐标点系列，电机将沿着由这些点生成的平滑曲线精准运动。

这一特性为您赋予了极高的创作自由度，能够设计出复杂多变的律动节奏。固件底层采用了 Catmull-Rom 样条插值算法，可将您输入的离散控制点拟合成一条顺滑、连续、无阶跃冲突的完美曲线，且轨迹能够绝对精确地穿过您所定义的每一个端点。

**如何使用：**

1.  **设定控制点：** 输入 `set-spline-points` 命令，后跟一组用空格分隔的浮点数（范围为 `0.0` 到 `1.0`，其中 `0.0` 代表完全收缩底端，`1.0` 代表完全伸展顶端）。
2.  **激活波形：** 输入 `set-wave spline` 命令，切换控制器至自定义样条波形模式。

**典型轨迹配置示例：**

*   **基础往复运动：** 最简的线性平滑往复。
    `set-spline-points 0 1`
*   **阶梯式快进慢退：** 极速推出后，分步骤阶梯式缓慢回抽。
    `set-spline-points 0 0 1 0.8 0.5 0.2`
*   **三角平滑波：** 匀速渐进上升与下降的匀称曲线。
    `set-spline-points 0 0.2 0.4 0.6 0.8 1.0 0.8 0.6 0.4 0.2`
*   **方波驻留模式：** 在底部保持静止，随后瞬间推进并在顶部保持静止。
    `set-spline-points 0 0 0 0 0 1 1 1 1 1`
*   **高频震颤模式：** 伴随细微回跳的抖动与震颤节律。
    `set-spline-points 0 0.2 0.1 0.4 0.3 0.6 0.5`

### HTTP API

固件内部提供了丰富的 HTTP RESTful API 接口，方便第三方应用程序进行程序化集成。所有接口均支持跨域资源共享（CORS），因此可以轻松地被部署在不同域名下的 Web App 访问调用。

#### `GET /config`

*   **请求方法：** `GET`
*   **接口描述：** 获取当前电机的完整控制配置。
*   **响应结构：** 返回一个包含当前运行配置的 JSON 对象。

```json
{
  "version": 42,
  "bpm": 60.0,
  "depth": 1.0,
  "depth_top": true,
  "reversed": false,
  "wave_func": "sine",
  "sharpness": 0.5,
  "spline_points": [0.0, 1.0],
  "paused": true,
  "paused_position": 0.5,
  "streaming": false
}
```

*   `version`（数字）：配置的单调递增因果时间戳。每次成功写入配置时递增；客户端应拒绝 `version` 回退的过期快照。可省略或传 `0`，由固件分配下一个版本号。
*   `bpm`（数字）：每分钟往复次数（Beats Per Minute）。直接控制运动周期的快慢。
*   `depth`（数字）：单次行程深度，范围从 `0.0`（完全静止不过推）至 `1.0`（全行程极限最大深度）。
*   `depth_top`（布尔值）：设定行程缩放的锚定方向。
    *   `true`：行程从完全收缩的最底层（`0.0`）起步，向顶端推至指定的 `depth`。例如，深度为 `0.8` 时，运动范围为 `[0.0, 0.8]`。
    *   `false`：行程从 `1.0 - depth` 起步，向完全伸展的最顶端（`1.0`）推出。例如，深度为 `0.8` 时，运动范围为 `[0.2, 1.0]`。
*   `reversed`（布尔值）：当设为 `true` 时，反转当前波形的往复方向。
*   `wave_func`（字符串）：选定的波形算法模式。固件生成器支持 `"sine"`（正弦波）、`"thrust"`（推力波）或 `"spline"`（自定义样条曲线波形）。网页端另有 `"funscript"` 与 `"macro"`（宏）模式：由客户端按时间轴下发 `set-config` / 暂停命令驱动；若把这些标签直接作为 `wave_func` 提交，固件会回退为正弦波。
*   `sharpness`（数字）：仅针对 `"thrust"` 推力波生效。用于控制冲刺推力的时间锐度，范围从 `0.01`（极速爆发最锐利）至 `0.99`（平顺缓慢最柔和）。
*   `spline_points`（浮点数数组）：当启用 `"spline"` 波形时，该数组用于定义自定义运动轨迹的归一化控制点坐标系列（范围均在 `0.0` 至 `1.0` 之间）。
*   `paused`（布尔值）：设为 `true` 时暂停电机运行，设为 `false` 时启动往复循环。
*   `paused_position`（数字）：指定电机处于暂停状态时安全停靠的归一化绝对坐标位置（范围 `0.0` 至 `1.0`）。
*   `streaming`（布尔值）：当设为 `true` 时，代表电机当前已开启实时流式控制模式，可通过 WebSocket (`/ws/command`) 接收上位机动态下发的连续轨迹点。

##### 控制宏（网页 UI + `ossm.py macro`）

**宏**是可分享的 JSON 时间轴控制序列（`start` / `stop` / `set`）。播放完全在客户端完成：网页「波形」中的 **宏** 面板，或 `./scripts/ossm.py macro <file.json>`，在每个 `at`（相对开始的毫秒）到达时下发对应 REST/BLE/串口命令。

```json
{
  "version": 1,
  "name": "Warm up",
  "loop": false,
  "instructions": [
    { "at": 0, "action": "set", "params": { "bpm": 40, "depth": 0.6, "wave_func": "sine" } },
    { "at": 0, "action": "start" },
    { "at": 15000, "action": "set", "params": { "bpm": 90, "wave_func": "thrust", "sharpness": 0.2 } },
    { "at": 60000, "action": "stop", "params": { "position": 0.0 } }
  ]
}
```

*   `set` 的 `params` 可含：`bpm`、`depth`、`depth_top`、`reversed`、`wave_func`（`sine`|`thrust`|`spline`）、`sharpness`、`spline_points`。
*   `stop` 可通过 `params.position`（0.0–1.0）可选停靠。
*   可在网页导入/导出，或用 `./scripts/ossm.py macro --init example.json` 生成示例文件。

#### `POST /config`

*   **请求方法：** `POST`
*   **接口描述：** 整体更新电机的控制配置。请注意，请求时务必发送完整的配置对象，目前系统不支持局部字段增量更新。若请求中的 `version` 早于设备当前配置，服务器返回 **409 Conflict**（`Stale causal version`）。
*   **请求消息体：** 数据结构与 `GET /config` 返回的 JSON 对象保持完全一致。
*   **响应消息体：** 成功应用后，返回已生效的配置 JSON 对象，包含递增后的 `version`。

#### `POST /paused`

*   **请求方法：** `POST`
*   **接口描述：** 专门用于实时控制电机的运行/暂停状态以及安全停靠位置。该接口非常适合在不开启完整往复循环的前提下，对电机轴进行精细的定位与微调。
*   **请求消息体：** 包含以下一个或多个可选字段的 JSON 对象：
    *   `paused`（布尔值）：设为 `true` 立即暂停电机，设为 `false` 恢复运行。
    *   `position`（数字）：直接指定安全停靠的归一化绝对坐标位置（范围 `0.0` 至 `1.0`）。
    *   `adjust`（数字）：在当前停靠位置的基础上进行相对偏移调整。例如，`0.1` 表示向前推进 10% 的行程，`-0.1` 表示向后回退 10%。
*   **响应消息体：** 成功应用后，返回最新的配置 JSON 对象。

**示例请求：**
```json
{
  "paused": true,
  "adjust": -0.05
}
```

#### `GET /state`

*   **请求方法：** `GET`
*   **接口描述：** 获取控制器的实时遥测状态数据。非常适合 UI 控制面板实时高频拉取电机的当前坐标、速度及流缓冲状态。
*   **响应消息体：** 包含完整的电机当前遥测数据的 JSON 对象。

```json
{
  "config": {
    "version": 42,
    "bpm": 60.0,
    "depth": 1.0,
    "depth_top": true,
    "reversed": false,
    "wave_func": "sine",
    "sharpness": 0.5,
    "spline_points": [0.0, 1.0],
    "paused": true,
    "paused_position": 0.5,
    "streaming": false
  },
  "t": 123.45,
  "x": 0.5,
  "y": 1.0,
  "shaped_y": 1.0,
  "position": 10000,
  "speed": 0.0,
  "stream": {
    "buffered": 0,
    "stream_time": 0.0,
    "underrun": false
  }
}
```

*   `config`：此时的完整 `MotorControllerConfig` 配置对象。
*   `t`：自运动启动以来的累计运行时间（秒）。
*   `x`：当前波形周期内的归一化相位，范围从 `0.0` 到 `1.0`。
*   `y`：波形生成器在当前相位的原始计算输出，范围从 `0.0` 到 `1.0`。
*   `shaped_y`：结合行程深度（depth）与运动方向（depth_top）映射缩放后的最终目标位置。
*   `position`：电机当前的绝对物理脉冲坐标（以驱动器原生编码器脉冲单位表示）。
*   `speed`：电机当前的速度估算值。
*   `stream`：外部流式实时控制的数据流状态（包含缓冲区剩余轨迹点总数 `buffered`、当前的流时间轴进度 `stream_time` 以及是否发生过缓冲区数据欠载 `underrun`）。

#### `GET /pin-config`

*   **请求方法：** `GET`
*   **接口描述：** 获取当前的 RS-485 Modbus GPIO 引脚映射及串行通信时序参数。
*   **响应消息体：** 表示硬件引脚与时序参数的 JSON 对象。

```json
{
  "modbus_tx": 18,
  "modbus_rx": 19,
  "modbus_de_re": 20,
  "modbus_timeout_ms": 0,
  "modbus_scan_delay_us": 0,
  "modbus_inter_frame_delay_us": 0,
  "ble_enabled": true,
  "modbus_debug": false
}
```

*   `modbus_tx`（数字）：分配给 Modbus DI/TX（发送）的 GPIO 引脚编号。
*   `modbus_rx`（数字）：分配给 Modbus RO/RX（接收）的 GPIO 引脚编号。
*   `modbus_de_re`（数字）：分配给 Modbus DE/RE（收发方向控制）的 GPIO 引脚编号。
*   `modbus_timeout_ms`（数字）：Modbus 单次总线响应超时时间（毫秒），`0` 表示由系统根据当前波特率自动匹配默认值（例如 115200 波特率默认为 **`10ms`**，最高支持 `1000`）。
*   `modbus_rx_timeout_us`（数字）：Modbus 字节间超时时间（微秒，$t_{1.5}$），`0` 表示自动（115200 波特率默认为 **`750µs`**，符合 >19200 bps 的 Modbus RTU 规范）。
*   `modbus_scan_delay_us`（数字）：Modbus 自动扫描时的帧间检测延迟（微秒），`0` 表示仅采用 Modbus 规范标准的 t3.5 间隙时序（最高支持 `200000`）。
*   `modbus_inter_frame_delay_us`（数字）：Modbus 正常通信时的帧间静默延迟（微秒，$t_{3.5}$），`0` 表示自动（115200 波特率默认为 `350µs`，确保完整的往返控制时延于 <3ms 以支撑 >300Hz 的电机控制刷新率）。
*   `ble_enabled`（布尔）：是否启用 BLE GATT 服务。
*   `modbus_debug`（布尔）：Modbus RX 诊断模式——**5 ms** 接收截止（与生产相同的 **256 B** UHCI DMA 缓冲），对每帧分类（`empty` / `short` / `exact` / `long` / `long_resync` / `leading_junk` / `parse_fail`），并在 USB 控制台打印 `MODBUS_DBG` TX/RX 十六进制。**需重启生效。** 会显著增加 USB 日志量；稳态电机 `ups` 通常仅比非调试低几个百分点（ESP32-C6 上约 310 vs 320 Hz）。控制台 **TX/RX 公平性**（`CLI_OUT_CH`、CLI/日志独立 USB 环形缓冲）使串口 CLI 在洪泛下仍可响应。可用 `./scripts/test_console_fairness.py` 验证；诊断后请关闭。也可通过 CLI `set pin.modbus_debug`、刷写器或 `ossm.py pins --modbus-debug` 设置。
*   `operating_mode`（字符串）：`"servo"`（默认 OSSM 运动控制）或 `"rtu_relay"`（Modbus RTU 桥接）。**需重启生效。**

#### `POST /pin-config`

*   **请求方法：** `POST`
*   **接口描述：** 更新 Modbus GPIO 引脚映射及通信时序配置。注意，修改硬件引脚映射或 `operating_mode` 后需要软重启微控制器方可生效。
*   **请求消息体：** 结构与 `GET /pin-config` 返回的 JSON 对象相同。
*   **响应消息体：** 成功更新后的配置 JSON 对象。

### RTU 中继模式

当 `operating_mode` 为 `"rtu_relay"`（设置面板、`POST /pin-config` 或串口 CLI `set pin.operating_mode rtu_relay`，然后重启）时，固件不运行电机控制环，而是作为 RS-485 Modbus RTU 桥：

* **Modbus TCP** 端口 **502**
* **WebSocket** `ws://<设备>/ws/modbus`（二进制完整 RTU 帧含 CRC）
* 网页前端显示中继提示并隐藏电机控制面板
* `release/motor-control.html` 支持 **Remote WebSocket** 连接
* 测试：`./scripts/test_modbus_relay.py --switch-mode`、`./scripts/test_modbus_tcp.py`

#### `POST /restart`

*   **请求方法：** `POST`
*   **接口描述：** 触发软复位指令，立即重启 ESP32 微控制器。
*   **响应消息体：** `{"ok":true}`

#### `GET /ws/command`

*   **请求方法：** `GET` (WebSocket 协议升级请求)
*   **接口描述：** 专为超低延迟、实时流式运动控制打造的 WebSocket 交互端点。允许第三方上位机、游戏互动插件或 VR 应用程序高频动态下发运动轨迹点（waypoints）以及实时订阅遥测流。
*   **协议与格式：** 同时兼容简单的扁平 JSON 指令帧（`{"cmd": "..."}`）以及标准规范的 JSON-RPC 2.0 请求消息（`{"jsonrpc": "2.0", "method": "...", "params": {...}, "id": 1}`）。如果在请求帧中指定了 `id` 标识，服务端将在执行完成后回应一个携带相同 `id` 的标准 JSON-RPC 回应结果（`{"jsonrpc": "2.0", "id": 1, "result": ...}`）。
*   **支持的 JSON-RPC 2.0 命令与处理机制**：
    *   **Append Waypoints（追加轨迹点）**：向底层流式缓冲区追加一组连续的目标运动轨迹点。
```json
{
  "jsonrpc": "2.0",
  "method": "append-waypoints",
  "params": [
    { "ts": 1000, "pos": 0.5, "vel": 0.2 },
    { "ts": 1050, "pos": 0.7 }
  ],
  "id": 1
}
```
        *   `ts`（数字）：发送端时间戳（毫秒）。
        *   `pos`（数字）：目标归一化坐标位置（范围 `0.0` 至 `1.0`）。
        *   `vel`（可选数字）：目标归一化运动速度（单位/秒）。
    *   **Set Waypoints（覆盖设置轨迹点）**：清空当前缓冲区剩余的旧轨迹点，并立即替换为新的一组目标轨迹点。可选在参数对象中传入 `reset-timestamp: true`，用于同步重置流式控制的时间基准，使新传入的第一个轨迹点重新作为时间轴起点。
```json
{
  "jsonrpc": "2.0",
  "method": "set-waypoints",
  "params": {
    "waypoints": [
      { "ts": 1000, "pos": 0.5, "vel": 0.2 }
    ],
    "reset-timestamp": true
  },
  "id": 2
}
```
    *   **Reset Timestamp（重置流时间戳）**：清空缓冲区，并强制重置底层流式计时器的基准 epoch，下一个接收到的轨迹点将重新作为时间原点。
```json
{
  "jsonrpc": "2.0",
  "method": "reset-timestamp",
  "id": 2
}
```
    *   **Status（查询流状态）**：实时查询当前底层流缓冲区的健康状态（`StreamStatus`，例如 `{"buffered": 0, "stream_time": 0.0, "underrun": false}`）。
```json
{
  "jsonrpc": "2.0",
  "method": "status",
  "id": 3
}
```
    *   **Ping（心跳探测）**：应用层链路存活心跳探测，服务端接收后将即时回复 `"pong"`。
```json
{
  "jsonrpc": "2.0",
  "method": "ping",
  "id": 4
}
```
    *   **Get State（获取当前状态）**：拉取系统当前完整的实时控制与遥测状态数据（`StateResponse`）。
```json
{
  "jsonrpc": "2.0",
  "method": "get-state",
  "id": 5
}
```
    *   **Set Config（动态更新配置）**：实时修改电机控制器的各项运行参数（`MotorControllerConfig`）。返回已生效的配置，包含递增后的 `version`。若 `params.version` 过期，返回 JSON-RPC 错误 `-32001`（`Stale causal version`）。
```json
{
  "jsonrpc": "2.0",
  "method": "set-config",
  "params": {
    "bpm": 60,
    "depth": 0.8,
    "paused": false
  },
  "id": 6
}
```
    *   **Subscribe State（订阅状态推流）**：开启服务端周期性自动推送的实时遥测数据流。
```json
{
  "jsonrpc": "2.0",
  "method": "subscribe-state",
  "params": {
    "interval_ms": 500
  },
  "id": 7
}
```
        *   订阅生效后，服务器会依照设定的时间间隔（`interval_ms`）自动向连接的客户端持续推送 JSON-RPC 格式的遥测数据帧：`{"jsonrpc": "2.0", "method": "state", "params": { ...StateResponse... }}`。再次发送 `subscribe-state` 可直接调整推流间隔，无需先取消订阅。
        *   **异步收发分离架构**：WebSocket 会话将底层 TCP 连结分离为独立读取（`ws_recv`）与写入（`ws_send`）任务，通过 Embassy 通道及堆分配字符串通信，确保高频推流不阻塞控制指令读入。
        *   **BLE 遥测**：GATT 对 `CHAR_STATE` 的读/通知使用**精简** JSON（必要时分块）。完整 `StateResponse`（循环遥测、历史数组等）请通过 `CHAR_RPC` 上的 JSON-RPC `get-state` 获取。BLE 的 `subscribe-state` 在 `CHAR_STATE` 上推送精简格式，而非 WiFi WebSocket 的完整载荷。
        *   **空闲自动断开**：为节省设备内存与 WebSocket 会话槽位（`WS_MAX` = 3 并发），嵌入式网页在无活动监听者时会于约 3 秒空闲后自动断开 WebSocket。
    *   **Unsubscribe State（取消状态订阅）**：停止服务端的周期性状态自动推送工作。
```json
{
  "jsonrpc": "2.0",
  "method": "unsubscribe-state",
  "id": 8
}
```

