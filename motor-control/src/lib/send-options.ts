import { ModbusRegister } from './registers'

export type WriteKind = 'u16' | 's16' | 's32' | 'custom79'

export interface SendTypeOption {
  id: number
  label: string
  description: string
  kind: WriteKind
  register?: ModbusRegister
  startRegister?: ModbusRegister
}

/** VB Combo_send_type / parameter_explain captions (Chinese-first). */
export const sendTypeOptions: SendTypeOption[] = [
  { id: 0, label: '0 mosbus使能', description: 'modbus使能 1:使能，0:禁止', kind: 'u16', register: ModbusRegister.MODBUS_ENABLE },
  { id: 1, label: '1 EN(使能)', description: '驱动器使能,0:禁止,1:使能', kind: 'u16', register: ModbusRegister.EN },
  { id: 2, label: '2 目标转速', description: '电机转速,单位r/min', kind: 's16', register: ModbusRegister.MODBUS_SPEED },
  { id: 3, label: '3 电机加速度', description: '速度模式: 加速度 范围0~30000(r/min)/s', kind: 'u16', register: ModbusRegister.MODBUS_A },
  { id: 4, label: '4 弱磁角度', description: '弱磁角度,范围0~200  单位 0.1°', kind: 'u16', register: ModbusRegister.MODBUS_SPEED_START },
  { id: 5, label: '5 速度环Kp', description: '速度控制KP参数,范围0~10000代表0.0到10.0倍', kind: 'u16', register: ModbusRegister.SPEED_KP },
  { id: 6, label: '6 速度环Ki', description: '速度环KI参数,范围5~2000 代表积分时间5ms~2000ms', kind: 'u16', register: ModbusRegister.SPEED_KI },
  { id: 7, label: '7 位置环Kp', description: '位置环KP参数,范围1~5000', kind: 'u16', register: ModbusRegister.POSITION_KP },
  { id: 8, label: '8 速度前馈', description: '速度前馈,换算算法327=1(V/KRPM)', kind: 'u16', register: ModbusRegister.SPEED_FEEDFORWARD },
  { id: 9, label: '9 DIR极性', description: '脉冲模式DIR极性,0:负逻辑,1:正逻辑', kind: 'u16', register: ModbusRegister.DIR_POLARITY },
  { id: 10, label: '10 电子齿轮分子', description: '电子齿轮分子', kind: 'u16', register: ModbusRegister.E_GEAR_NUMERATOR },
  { id: 11, label: '11 电子齿轮分母', description: '电子齿轮分母', kind: 'u16', register: ModbusRegister.E_GEAR_DENOMINATOR },
  { id: 12, label: '12 参数保存', description: '参数保存标志,0:不保存  1:保存', kind: 'u16', register: ModbusRegister.MODBUS_SAVE_FLAG },
  { id: 13, label: '13 PU(步数)', description: 'PU 电机走的总步数', kind: 's32', startRegister: ModbusRegister.PU_LO },
  { id: 14, label: '14 设备地址', description: '修改设备地址,写入需要改成的地址', kind: 'u16', register: ModbusRegister.DEVICE_ADDRESS },
  { id: 15, label: '15 温度补偿', description: '修改温度', kind: 'custom79' },
  { id: 16, label: '16 静态最大输出', description: '静态最大输出，范围0~600，代表0~60.0%', kind: 'u16', register: ModbusRegister.SPEED_FILTER },
  { id: 17, label: '17 特殊功能', description: '范围10~32768，对应0.1~360°，2为编码器跟随模式', kind: 'u16', register: ModbusRegister.POSITION_FORWARD },
  { id: 18, label: '18 PU(总步数)', description: 'PU 电机走的总步数', kind: 's32', startRegister: ModbusRegister.PU_TOTAL_LO },
]
