# OSSM-Rust Firmware

This repository contains the firmware for an OSSM controller, written in Rust. It's designed to run on ESP32-C6 microcontrollers and control a 57AIM30 integrated servo motor.

This guide is intended for users who may not have a technical background. Please follow the steps carefully.

## Part 1: Required Hardware

Here is a list of components you will need to build the controller.

| Component | Description | Notes |
| --- | --- | --- |
| **Microcontroller** | ESP32-C6 Development Board | Any ESP32-C6 board with a USB-C connector should work. |
| **Motor** | 57AIM30 Integrated Servo Motor | **Important:** Make sure to get the `57AIM30` model, not the `57AIM30H`. The `57AIM30` has a rated speed of 1500 RPM and 0.96 Nm of torque, which is ideal for this application. |
| **RS485 Transceiver**| MAX3485 Module | This is used for communication between the ESP32 and the motor. |
| **Power Supply (Motor)** | 24V DC Power Adapter | To power the servo motor. The connector type should be compatible with your DC jack. |
| **Power Supply (ESP32)**| 5V USB Charger | Any standard USB phone charger with a USB-C cable will work. |
| **Cables & Connectors** | - DC 5.5mm female jack<br>- PHB2.0 2x5 pin connector cable<br>- Jumper wires | The DC jack is for connecting the 24V power supply to the motor. The PHB2.0 cable connects to the motor's communication port. You will need to do some soldering. |

## Part 2: Wiring

Connecting the components correctly is crucial. Please double-check all connections before powering anything on.

Here is a breakdown of the wiring:

### Motor Power Connection (Front 6-pin terminal)

| Motor (+V) | --> | 24V Power Supply (Positive `+`) |
| --- | --- | --- |
| Motor (GND) | --> | 24V Power Supply (Negative `-`) |

### Communication and Logic Power (Motor's back 2x5 pin terminal & ESP32)

First, connect the motor to the MAX3485 RS485 transceiver:

| Motor (485A) | --> | MAX3485 (A) |
| --- | --- | --- |
| Motor (485B) | --> | MAX3485 (B) |

Next, connect the ESP32-C6 to the MAX3485 and the motor:

| ESP32-C6 Pin | --> | Component Pin |
| --- | --- | --- |
| 5V | --> | Motor (5V) |
| GND | --> | Motor (COM) |
| 3.3V | --> | MAX3485 (VCC/3.3V) |
| GND | --> | MAX3485 (GND) |
| GPIO 18 (TX) | --> | MAX3485 (DI/TX) |
| GPIO 19 (RX) | --> | MAX3485 (RO/RX) |
| GPIO 20 | --> | MAX3485 (DE/RE) |

Finally, power the ESP32-C6 board:

| ESP32-C6 Type-C Port | --> | 5V USB Charger |
| --- | --- | --- |

**Note on GPIO pins**: The firmware uses GPIO 18, 19, and 20 by default for Modbus communication. If you use different pins, you will need to configure them later via serial commands.

## Part 3: Flashing the Firmware

You don't need to build the firmware from source. Pre-compiled binary files will be available in the **Releases** section of this GitHub repository.

We will use a web-based tool to flash the firmware onto your ESP32-C6.

1.  Download the latest `firmware.bin` file from the Releases page of this project.
2.  Connect your ESP32-C6 to your computer using a USB-C cable.
3.  Open your web browser (Google Chrome or Microsoft Edge are recommended) and go to: **[https://espressif.github.io/esptool-js/](https://espressif.github.io/esptool-js/)**
4.  Click the **"Connect"** button. A popup will appear asking you to select the serial port for your ESP32. It will usually be named `COMx` on Windows or `/dev/ttyUSBx` on Linux/macOS.
5.  Once connected, you will see a table with "Flash Address" and "File" columns.
6.  In the "Flash Address" field, enter `0x0` (this is the offset for the merged firmware image).
7.  Click **"Choose a file..."** and select the `firmware.bin` file you downloaded.
8.  Click the **"Program"** button to start flashing.
9.  Wait for the process to complete. You should see a "Finished" message in the log.

## Part 4: Configuration

After flashing, you need to configure the device to connect to your WiFi network. You'll do this by sending commands over a serial connection. The tool used for flashing can also be used as a serial monitor.

1.  Keep the ESP32-C6 connected to your computer and stay on the flashing tool page. If you have closed it, you can open it again: **[https://espressif.github.io/esptool-js/](https://espressif.github.io/esptool-js/)**
2.  If you are not connected, click **"Connect"** and select the same serial port you used for flashing.
3.  The console is located at the bottom of the page. You should see log messages from the device. Press `Enter` in the input box to make sure the connection is working.
4.  To configure WiFi, type the following commands one by one, replacing `<your_ssid>` and `<your_password>` with your actual WiFi network name and password. Press `Enter` after each command.

    ```
    set_wifi_ssid <your_ssid>
    set_wifi_password <your_password>
    ```

6.  After setting the SSID and password, you need to restart the ESP32-C6. You can do this by pressing the `RST` or `EN` button on the board, or by unplugging and plugging it back in.
7.  The device will now connect to your WiFi network. In the serial monitor, you should see a message indicating it has connected and received an IP address. Note down this IP address.

## Part 5: Usage

Once the device is on your network, you can control it through a web interface or an API. The serial commands are also available for control.

### Web Interface

The firmware hosts a simple web interface. Open a browser on a device connected to the same WiFi network and navigate to the IP address of your ESP32 (the one you noted down earlier).

Example: `http://192.168.1.123`

From here, you can control the motor's functions.

### Serial Commands

You can also control the motor using the serial monitor. Here is a list of available commands. This is useful for testing and debugging.

```
help                           - Show this help message
set_wifi_ssid <ssid>           - Set WiFi SSID
set_wifi_password <password>   - Set WiFi password
get_pin_configuration          - Get pin configuration in JSON format
set_pin_modbus_tx <pin>        - Set Modbus TX pin
set_pin_modbus_rx <pin>        - Set Modbus RX pin
set_pin_modbus_de_re <pin>     - Set Modbus DE/RE pin
get_motor_config               - Get motor config in JSON format
set_motor_config <json>        - Set motor config from a JSON string
pause                          - Pause the motor
start                          - Start the motor
set_bpm <bpm>                  - Set motor BPM
set_wave <sine|thrust|spline>  - Set motor waveform
set_paused_position <position> - Set motor position when paused (0.0 to 1.0)
set_depth <depth>              - Set motor stroke depth (0.0 to 1.0)
set_depth_top <true|false>     - Set depth direction
set_sharpness <sharpness>      - Set sharpness for thrust wave (0.01 to 0.99)
set_spline_points <p1> <p2>... - Set points for spline wave (0.0 to 1.0)
```

### Advanced Control: The Spline Wave

The `spline` wave is a powerful feature for creating custom motion patterns. Instead of being limited to predefined motions like `sine` or `thrust`, you can define a completely custom movement by providing a sequence of points. The motor will then travel through these points smoothly.

This gives you the creative freedom to design intricate and varied patterns. The firmware uses a technique called Catmull-Rom spline interpolation to generate a smooth, continuous curve that passes exactly through each point you've defined.

**How to use it:**

1.  **Set the points:** Use the `set_spline_points` command, followed by a space-separated list of numbers between 0.0 (fully retracted) and 1.0 (fully extended).
2.  **Activate the wave:** Use the `set_wave spline` command to switch to your custom pattern.

**Examples:**

*   **Simple Stroke:** A basic linear movement.
    `set_spline_points 0 1`
*   **Thrust:** A rapid forward motion followed by a stepped retraction.
    `set_spline_points 0 0 1 0.8 0.5 0.2`
*   **Triangle:** A smooth ramping up and down.
    `set_spline_points 0 0.2 0.4 0.6 0.8 1.0 0.8 0.6 0.4 0.2`
*   **Square Wave:** Holds at the start, then instantly moves and holds at the end.
    `set_spline_points 0 0 0 0 0 1 1 1 1 1`
*   **Vibration:** A jittery, vibrational motion.
    `set_spline_points 0 0.2 0.1 0.4 0.3 0.6 0.5`

### HTTP API

The firmware also provides an HTTP API for programmatic control. All endpoints support CORS, so they can be accessed from web applications running on different domains.

#### `GET /config`

*   **Method:** `GET`
*   **Description:** Retrieves the current motor configuration.
*   **Response Body:** A JSON object representing the motor controller's configuration.

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

*   `bpm` (number): Beats per minute. Controls the speed of the motion cycle.
*   `depth` (number): The stroke depth, from 0.0 (no movement) to 1.0 (full range).
*   `depth_top` (boolean): Determines the direction of the stroke.
    *   `true`: The stroke moves from the fully retracted position (0.0) to the specified `depth`. For example, a depth of 0.8 would move in the range [0.0, 0.8].
    *   `false`: The stroke moves from `1.0 - depth` to the fully extended position (1.0). For example, a depth of 0.8 would move in the range [0.2, 1.0].
*   `reversed` (boolean): When `true`, reverses the direction of the waveform.
*   `wave_func` (string): The motion pattern. Can be `"sine"`, `"thrust"`, or `"spline"`.
*   `sharpness` (number): Only affects the `"thrust"` waveform. Controls the duration of the thrust, from 0.01 (sharpest) to 0.99 (smoothest).
*   `spline_points` (array of numbers): An array of points (0.0 to 1.0) that define the custom motion path for the `"spline"` waveform.
*   `paused` (boolean): `true` to pause the motor, `false` to run it.
*   `paused_position` (number): The position (0.0 to 1.0) the motor will hold when paused.

#### `POST /config`

*   **Method:** `POST`
*   **Description:** Updates the motor configuration. You must send a full configuration object, as partial updates are not supported.
*   **Request Body:** A JSON object with the same structure as the `GET /config` response.
*   **Response Body:** The updated configuration as a JSON object.

#### `POST /paused`

*   **Method:** `POST`
*   **Description:** Controls the motor's state when paused. This is useful for making fine adjustments to the position without starting a full motion cycle.
*   **Request Body:** A JSON object with one or more of the following optional fields:
    *   `paused` (boolean): Set to `true` to pause the motor, `false` to resume.
    *   `position` (number): Sets the absolute paused position (from 0.0 to 1.0).
    *   `adjust` (number): Adjusts the position relatively. For example, `0.1` moves it forward by 10%, and `-0.1` moves it back.
*   **Response Body:** The updated configuration as a JSON object.

**Example Request:**
```json
{
  "paused": true,
  "adjust": -0.05
}
```

#### `GET /state`

*   **Method:** `GET`
*   **Description:** Retrieves the current real-time state of the motor. This is useful for UIs that need to display the motor's live position and other metrics.
*   **Response Body:** A JSON object containing the motor's complete current state.

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

*   `config`: The full `MotorControllerConfig` object at this moment.
*   `t`: Time offset in seconds since the motion started.
*   `x`: The current phase of the waveform, from 0.0 to 1.0.
*   `y`: The raw output of the waveform generator, from 0.0 to 1.0.
*   `shaped_y`: The waveform output after depth and direction have been applied.
*   `position`: The current absolute position of the motor in its native units.
*   `speed`: The current speed of the motor.
