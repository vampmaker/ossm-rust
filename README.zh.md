# OSSM-Rust 固件

本仓库包含用 Rust 编写的 OSSM 控制器固件。它设计用于在 ESP32-C6 微控制器上运行，并控制 57AIM30 集成伺服电机。

本指南面向可能没有技术背景的用户。请仔细按照步骤操作。

## 第一部分：所需硬件

以下是构建控制器所需的组件列表。

| 组件 | 描述 | 备注 |
| --- | --- | --- |
| **微控制器** | ESP32-C6 开发板 | 任何带有 USB-C 接口的 ESP32-C6 开发板都可以使用。 |
| **电机** | 57AIM30 集成伺服电机 | **重要：** 请确保购买 `57AIM30` 型号，而不是 `57AIM30H`。`57AIM30` 的额定转速为 1500 RPM，扭矩为 0.96 Nm，非常适合此应用。 |
| **RS485 收发器**| MAX3485 模块 | 用于 ESP32 和电机之间的通信。 |
| **电源（电机）** | 24V 直流电源适配器 | 为伺服电机供电。连接器类型应与您的 DC 插座兼容。 |
| **电源（ESP32）**| 5V USB 充电器 | 任何标准的 USB 手机充电器配 USB-C 线都可以使用。 |
| **线缆和连接器** | - DC 5.5mm 母头插座<br>- PHB2.0 2x5 针连接线<br>- 杜邦线 | DC 插座用于将 24V 电源连接到电机。PHB2.0 线缆连接到电机的通信端口。您需要进行一些焊接工作。 |

## 第二部分：接线

正确连接组件至关重要。在打开任何电源之前，请仔细检查所有连接。

以下是接线详情：

### 电机电源连接（前端 6 针端子）

| 电机 (+V) | --> | 24V 电源（正极 `+`） |
| --- | --- | --- |
| 电机 (GND) | --> | 24V 电源（负极 `-`） |

### 通信和逻辑电源（电机后端 2x5 针端子和 ESP32）

首先，将电机连接到 MAX3485 RS485 收发器：

| 电机 (485A) | --> | MAX3485 (A) |
| --- | --- | --- |
| 电机 (485B) | --> | MAX3485 (B) |

接下来，将 ESP32-C6 连接到 MAX3485 和电机：

| ESP32-C6 引脚 | --> | 组件引脚 |
| --- | --- | --- |
| 5V | --> | 电机 (5V) |
| GND | --> | 电机 (COM) |
| 3.3V | --> | MAX3485 (VCC/3.3V) |
| GND | --> | MAX3485 (GND) |
| GPIO 18 (TX) | --> | MAX3485 (DI/TX) |
| GPIO 19 (RX) | --> | MAX3485 (RO/RX) |
| GPIO 20 | --> | MAX3485 (DE/RE) |

最后，为 ESP32-C6 开发板供电：

| ESP32-C6 Type-C 接口 | --> | 5V USB 充电器 |
| --- | --- | --- |

**关于 GPIO 引脚的说明**：固件默认使用 GPIO 18、19 和 20 进行 Modbus 通信。如果您使用不同的引脚，您需要稍后通过串口命令进行配置。

## 第三部分：烧录固件

您不需要从源代码构建固件。预编译的二进制文件将在此 GitHub 仓库的 **Releases** 部分提供。

我们将使用基于 Web 的工具将固件烧录到您的 ESP32-C6。

1.  从本项目的 Releases 页面下载最新的 `firmware.bin` 文件。
2.  使用 USB-C 线将 ESP32-C6 连接到您的计算机。
3.  打开您的网络浏览器（推荐使用 Google Chrome 或 Microsoft Edge），访问：**[https://espressif.github.io/esptool-js/](https://espressif.github.io/esptool-js/)**
4.  点击 **"Connect"** 按钮。会弹出一个窗口，要求您选择 ESP32 的串口。在 Windows 上通常命名为 `COMx`，在 Linux/macOS 上为 `/dev/ttyUSBx`。
5.  连接成功后，您将看到一个包含"Flash Address"（烧录地址）和"File"（文件）列的表格。
6.  在"Flash Address"字段中，输入 `0x0`（这是合并固件镜像的偏移量）。
7.  点击 **"Choose a file..."** 并选择您下载的 `firmware.bin` 文件。
8.  点击 **"Program"** 按钮开始烧录。
9.  等待过程完成。您应该会在日志中看到 "Finished" 消息。

## 第四部分：配置

烧录完成后，您需要配置设备以连接到您的 WiFi 网络。您将通过串口连接发送命令来完成此操作。用于烧录的工具也可以用作串口监视器。

1.  保持 ESP32-C6 连接到您的计算机，并停留在烧录工具页面。如果您已关闭它，可以重新打开：**[https://espressif.github.io/esptool-js/](https://espressif.github.io/esptool-js/)**
2.  如果您未连接，请点击 **"Connect"** 并选择您用于烧录的同一串口。
3.  控制台位于页面底部。您应该会看到来自设备的日志消息。在输入框中按 `Enter` 以确保连接正常工作。
4.  要配置 WiFi，逐个输入以下命令，将 `<your_ssid>` 和 `<your_password>` 替换为您实际的 WiFi 网络名称和密码。每条命令后按 `Enter`。

    ```
    set_wifi_ssid <your_ssid>
    set_wifi_password <your_password>
    ```

6.  设置 SSID 和密码后，您需要重启 ESP32-C6。您可以通过按下开发板上的 `RST` 或 `EN` 按钮来完成，或者拔出并重新插入。
7.  设备现在将连接到您的 WiFi 网络。在串口监视器中，您应该会看到一条消息，指示它已连接并收到 IP 地址。记下此 IP 地址。

## 第五部分：使用

一旦设备连接到您的网络，您可以通过 Web 界面或 API 来控制它。串口命令也可用于控制。

### Web 界面

固件托管了一个简单的 Web 界面。在连接到同一 WiFi 网络的设备上打开浏览器，然后导航到您的 ESP32 的 IP 地址（您之前记下的地址）。

示例：`http://192.168.1.123`

从这里，您可以控制电机的功能。

### 串口命令

您还可以使用串口监视器来控制电机。以下是可用命令列表。这对于测试和调试很有用。

```
help                           - 显示此帮助消息
set_wifi_ssid <ssid>           - 设置 WiFi SSID
set_wifi_password <password>   - 设置 WiFi 密码
get_pin_configuration          - 以 JSON 格式获取引脚配置
set_pin_modbus_tx <pin>        - 设置 Modbus TX 引脚
set_pin_modbus_rx <pin>        - 设置 Modbus RX 引脚
set_pin_modbus_de_re <pin>     - 设置 Modbus DE/RE 引脚
get_motor_config               - 以 JSON 格式获取电机配置
set_motor_config <json>        - 从 JSON 字符串设置电机配置
pause                          - 暂停电机
start                          - 启动电机
set_bpm <bpm>                  - 设置电机 BPM
set_wave <sine|thrust|spline>  - 设置电机波形
set_paused_position <position> - 设置电机暂停时的位置（0.0 到 1.0）
set_depth <depth>              - 设置电机行程深度（0.0 到 1.0）
set_depth_top <true|false>     - 设置深度方向
set_sharpness <sharpness>      - 设置推力波的锐度（0.01 到 0.99）
set_spline_points <p1> <p2>... - 设置样条波的点（0.0 到 1.0）
```

### 高级控制：样条波

`spline` 波是一个强大的功能，用于创建自定义运动模式。您不再局限于预定义的运动，如 `sine` 或 `thrust`，而是可以通过提供一系列点来定义完全自定义的运动。电机将平滑地通过这些点移动。

这为您提供了设计复杂多样模式的创作自由。固件使用一种称为 Catmull-Rom 样条插值的技术来生成平滑、连续的曲线，该曲线恰好通过您定义的每个点。

**如何使用：**

1.  **设置点：** 使用 `set_spline_points` 命令，后跟一系列用空格分隔的数字，范围从 0.0（完全收缩）到 1.0（完全伸展）。
2.  **激活波形：** 使用 `set_wave spline` 命令切换到您的自定义模式。

**示例：**

*   **简单行程：** 基本的线性运动。
    `set_spline_points 0 1`
*   **推力：** 快速前进运动，然后阶梯式收缩。
    `set_spline_points 0 0 1 0.8 0.5 0.2`
*   **三角形：** 平滑地上升和下降。
    `set_spline_points 0 0.2 0.4 0.6 0.8 1.0 0.8 0.6 0.4 0.2`
*   **方波：** 在开始处保持，然后立即移动并在结束处保持。
    `set_spline_points 0 0 0 0 0 1 1 1 1 1`
*   **振动：** 抖动、振动的运动。
    `set_spline_points 0 0.2 0.1 0.4 0.3 0.6 0.5`

### HTTP API

固件还提供了用于程序化控制的 HTTP API。所有端点都支持 CORS，因此可以从运行在不同域上的 Web 应用程序访问。

#### `GET /config`

*   **方法：** `GET`
*   **描述：** 检索当前电机配置。
*   **响应体：** 表示电机控制器配置的 JSON 对象。

```json
{
  "bpm": 60.0,
  "depth": 1.0,
  "depth_top": true,
  "reversed": false,
  "wave_func": "sine",
  "sharpness": 0.5,
  "spline_points": [0.0, 1.0],
  "paused": true,
  "paused_position": 0.5
}
```

*   `bpm`（数字）：每分钟节拍数。控制运动周期的速度。
*   `depth`（数字）：行程深度，从 0.0（无运动）到 1.0（全范围）。
*   `depth_top`（布尔值）：确定行程的方向。
    *   `true`：行程从完全收缩位置（0.0）移动到指定的 `depth`。例如，深度为 0.8 将在范围 [0.0, 0.8] 内移动。
    *   `false`：行程从 `1.0 - depth` 移动到完全伸展位置（1.0）。例如，深度为 0.8 将在范围 [0.2, 1.0] 内移动。
*   `reversed`（布尔值）：当为 `true` 时，反转波形的方向。
*   `wave_func`（字符串）：运动模式。可以是 `"sine"`、`"thrust"` 或 `"spline"`。
*   `sharpness`（数字）：仅影响 `"thrust"` 波形。控制推力的持续时间，从 0.01（最锐利）到 0.99（最平滑）。
*   `spline_points`（数字数组）：定义 `"spline"` 波形的自定义运动路径的点数组（0.0 到 1.0）。
*   `paused`（布尔值）：`true` 暂停电机，`false` 运行电机。
*   `paused_position`（数字）：电机暂停时将保持的位置（0.0 到 1.0）。

#### `POST /config`

*   **方法：** `POST`
*   **描述：** 更新电机配置。您必须发送完整的配置对象，因为不支持部分更新。
*   **请求体：** 与 `GET /config` 响应具有相同结构的 JSON 对象。
*   **响应体：** 更新后的配置作为 JSON 对象。

#### `POST /paused`

*   **方法：** `POST`
*   **描述：** 控制电机暂停时的状态。这对于在不启动完整运动周期的情况下对位置进行微调很有用。
*   **请求体：** 具有以下一个或多个可选字段的 JSON 对象：
    *   `paused`（布尔值）：设置为 `true` 以暂停电机，`false` 以恢复。
    *   `position`（数字）：设置绝对暂停位置（从 0.0 到 1.0）。
    *   `adjust`（数字）：相对调整位置。例如，`0.1` 向前移动 10%，`-0.1` 向后移动。
*   **响应体：** 更新后的配置作为 JSON 对象。

**示例请求：**
```json
{
  "paused": true,
  "adjust": -0.05
}
```

#### `GET /state`

*   **方法：** `GET`
*   **描述：** 检索电机的当前实时状态。这对于需要显示电机实时位置和其他指标的 UI 很有用。
*   **响应体：** 包含电机完整当前状态的 JSON 对象。

```json
{
  "config": {
    "bpm": 60.0,
    "depth": 1.0,
    "depth_top": true,
    "reversed": false,
    "wave_func": "sine",
    "sharpness": 0.5,
    "spline_points": [0.0, 1.0],
    "paused": true,
    "paused_position": 0.5
  },
  "t": 123.45,
  "x": 0.5,
  "y": 1.0,
  "shaped_y": 1.0,
  "position": 10000,
  "speed": 0.0
}
```

*   `config`：此时的完整 `MotorControllerConfig` 对象。
*   `t`：自运动开始以来的时间偏移（秒）。
*   `x`：波形的当前相位，从 0.0 到 1.0。
*   `y`：波形生成器的原始输出，从 0.0 到 1.0。
*   `shaped_y`：应用深度和方向后的波形输出。
*   `position`：电机的当前绝对位置（以其原生单位表示）。
*   `speed`：电机的当前速度。

