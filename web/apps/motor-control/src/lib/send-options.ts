import { ModbusRegister } from './registers'

export type WriteKind = 'u16' | 's16' | 's32' | 'custom79'

export interface SendTypeMeta {
  id: number
  kind: WriteKind
  register?: ModbusRegister
  startRegister?: ModbusRegister
}

export interface SendTypeOption extends SendTypeMeta {
  label: string
  description: string
}

/** VB Combo_send_type register metadata (labels live in locales). */
export const sendTypeMeta: SendTypeMeta[] = [
  { id: 0, kind: 'u16', register: ModbusRegister.MODBUS_ENABLE },
  { id: 1, kind: 'u16', register: ModbusRegister.EN },
  { id: 2, kind: 's16', register: ModbusRegister.MODBUS_SPEED },
  { id: 3, kind: 'u16', register: ModbusRegister.MODBUS_A },
  { id: 4, kind: 'u16', register: ModbusRegister.MODBUS_SPEED_START },
  { id: 5, kind: 'u16', register: ModbusRegister.SPEED_KP },
  { id: 6, kind: 'u16', register: ModbusRegister.SPEED_KI },
  { id: 7, kind: 'u16', register: ModbusRegister.POSITION_KP },
  { id: 8, kind: 'u16', register: ModbusRegister.SPEED_FEEDFORWARD },
  { id: 9, kind: 'u16', register: ModbusRegister.DIR_POLARITY },
  { id: 10, kind: 'u16', register: ModbusRegister.E_GEAR_NUMERATOR },
  { id: 11, kind: 'u16', register: ModbusRegister.E_GEAR_DENOMINATOR },
  { id: 12, kind: 'u16', register: ModbusRegister.MODBUS_SAVE_FLAG },
  { id: 13, kind: 's32', startRegister: ModbusRegister.PU_LO },
  { id: 14, kind: 'u16', register: ModbusRegister.DEVICE_ADDRESS },
  { id: 15, kind: 'custom79' },
  { id: 16, kind: 'u16', register: ModbusRegister.SPEED_FILTER },
  { id: 17, kind: 'u16', register: ModbusRegister.POSITION_FORWARD },
  { id: 18, kind: 's32', startRegister: ModbusRegister.PU_TOTAL_LO },
]

type TranslateFn = (key: string, ...args: any[]) => any

export function getSendTypeOptions(t: TranslateFn): SendTypeOption[] {
  return sendTypeMeta.map((meta) => ({
    ...meta,
    label: String(t(`sendType.${meta.id}.label`)),
    description: String(t(`sendType.${meta.id}.description`)),
  }))
}
