export enum ModbusRegister {
  MODBUS_ENABLE = 0,
  EN = 1,
  MODBUS_SPEED = 2,
  MODBUS_A = 3,
  MODBUS_SPEED_START = 4,
  SPEED_KP = 5,
  SPEED_KI = 6,
  POSITION_KP = 7,
  SPEED_FEEDFORWARD = 8,
  DIR_POLARITY = 9,
  E_GEAR_NUMERATOR = 10,
  E_GEAR_DENOMINATOR = 11,
  PU_LO = 12,
  PU_HI = 13,
  WARNING_CODE = 14,
  SYSTEM_I_DC = 15,
  SPEED_CURRENT = 16,
  SYSTEM_V = 17,
  SYSTEM_T = 18,
  SYSTEM_V_PWM = 19,
  MODBUS_SAVE_FLAG = 20,
  DEVICE_ADDRESS = 21,
  PU_TOTAL_LO = 22,
  PU_TOTAL_HI = 23,
  SPEED_FILTER = 24,
  POSITION_FORWARD = 25
}

export const WARNING_CODES: Record<number, string> = {
  0x00: "Normal Operation / 运行正常",
  0x10: "Drive Overheat (>70°C) / 驱动器过热( >70℃ )",
  0x20: "Flash Write Failed / 写入flash失败",
  0x11: "Drive Overheat Stop (>90°C) / 驱动器过热( >90℃),驱动器停机",
  0x12: "Overcurrent / 驱动器过流报警",
  0x13: "Undervoltage / 驱动器欠压报警",
  0x14: "Stall / 驱动器失速报警",
  0x15: "Pulse Frequency Too High / 驱动器脉冲输入频率过高"
};

export function getWarningMessage(code: number): string {
  return WARNING_CODES[code] || `Unknown Error (0x${code.toString(16)})`;
}

// Convert unsigned 16-bit to signed 16-bit
export function toSigned16(val: number): number {
  return val > 32767 ? val - 65536 : val;
}

// Convert signed 16-bit to unsigned 16-bit
export function toUnsigned16(val: number): number {
  return val < 0 ? val + 65536 : val;
}

// Combine two 16-bit registers into a signed 32-bit integer (Little Endian words)
export function toSigned32(lo: number, hi: number): number {
  const unsigned = (hi << 16) | lo;
  return unsigned > 0x7FFFFFFF ? unsigned - 0x100000000 : unsigned;
}

// Split a signed 32-bit integer into two 16-bit registers [lo, hi]
export function fromSigned32(val: number): [number, number] {
  const unsigned = val < 0 ? val + 0x100000000 : val;
  return [unsigned & 0xFFFF, (unsigned >> 16) & 0xFFFF];
}

// Scaling helpers
export const Scaling = {
  current: (val: number) => toSigned16(val) / 2000.0,      // Amps
  voltage: (val: number) => toSigned16(val) / 327.0,       // Volts
  pwm: (val: number) => toSigned16(val) / 327.0,           // %
  speed: (val: number) => toSigned16(val) / 10.0,          // r/min
  temperature: (val: number) => val > 32760 ? 32760 : val, // °C
  feedforward: (val: number) => val / 327.0                // V/KRPM
};

export interface DriveState {
  modbusEnable: number;
  en: number;
  targetSpeed: number;
  acceleration: number;
  speedStart: number;
  speedKp: number;
  speedKi: number;
  positionKp: number;
  speedFeedforward: number;
  /** Raw register value before /327 scaling (for display of feedforward integer). */
  speedFeedforwardRaw: number;
  dirPolarity: number;
  eGearNumerator: number;
  eGearDenominator: number;
  pu: number;
  warningCode: number;
  currentA: number;
  speedRmin: number;
  voltageV: number;
  temperatureC: number;
  pwmPercent: number;
  /** Signed raw holding-register values for VB-style waveform (/32768). */
  currentRaw: number;
  speedRaw: number;
  voltageRaw: number;
  pwmRaw: number;
  saveFlag: number;
  puTotal: number;
  speedFilter: number;
  positionForward: number;
}

export function parseRegistersToState(registers: Uint16Array): DriveState {
  const currentRaw = toSigned16(registers[ModbusRegister.SYSTEM_I_DC]!);
  const speedRaw = toSigned16(registers[ModbusRegister.SPEED_CURRENT]!);
  const voltageRaw = toSigned16(registers[ModbusRegister.SYSTEM_V]!);
  const pwmRaw = toSigned16(registers[ModbusRegister.SYSTEM_V_PWM]!);
  const speedFeedforwardRaw = registers[ModbusRegister.SPEED_FEEDFORWARD]!;
  return {
    modbusEnable: registers[ModbusRegister.MODBUS_ENABLE]!,
    en: registers[ModbusRegister.EN]!,
    targetSpeed: toSigned16(registers[ModbusRegister.MODBUS_SPEED]!),
    acceleration: registers[ModbusRegister.MODBUS_A]!,
    speedStart: registers[ModbusRegister.MODBUS_SPEED_START]!,
    speedKp: registers[ModbusRegister.SPEED_KP]!,
    speedKi: registers[ModbusRegister.SPEED_KI]!,
    positionKp: registers[ModbusRegister.POSITION_KP]!,
    speedFeedforward: Scaling.feedforward(speedFeedforwardRaw),
    speedFeedforwardRaw,
    dirPolarity: registers[ModbusRegister.DIR_POLARITY]!,
    eGearNumerator: registers[ModbusRegister.E_GEAR_NUMERATOR]!,
    eGearDenominator: registers[ModbusRegister.E_GEAR_DENOMINATOR]!,
    pu: toSigned32(registers[ModbusRegister.PU_LO]!, registers[ModbusRegister.PU_HI]!),
    warningCode: registers[ModbusRegister.WARNING_CODE]!,
    currentA: Scaling.current(registers[ModbusRegister.SYSTEM_I_DC]!),
    speedRmin: Scaling.speed(registers[ModbusRegister.SPEED_CURRENT]!),
    voltageV: Scaling.voltage(registers[ModbusRegister.SYSTEM_V]!),
    temperatureC: Scaling.temperature(registers[ModbusRegister.SYSTEM_T]!),
    pwmPercent: Scaling.pwm(registers[ModbusRegister.SYSTEM_V_PWM]!),
    currentRaw,
    speedRaw,
    voltageRaw,
    pwmRaw,
    saveFlag: registers[ModbusRegister.MODBUS_SAVE_FLAG]!,
    puTotal: toSigned32(registers[ModbusRegister.PU_TOTAL_LO]!, registers[ModbusRegister.PU_TOTAL_HI]!),
    speedFilter: registers[ModbusRegister.SPEED_FILTER]!,
    positionForward: registers[ModbusRegister.POSITION_FORWARD]!
  };
}
