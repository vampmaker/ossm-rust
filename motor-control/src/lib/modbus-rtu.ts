/**
 * Modbus RTU implementation for Web Serial
 * Includes CRC-16 calculation and frame building/parsing
 */

// CRC-16/MODBUS calculation
export function calculateCRC(buffer: Uint8Array, length: number = buffer.length): number {
  let crc = 0xFFFF;
  for (let pos = 0; pos < length; pos++) {
    crc ^= buffer[pos]!;
    for (let i = 8; i !== 0; i--) {
      if ((crc & 0x0001) !== 0) {
        crc >>= 1;
        crc ^= 0xA001;
      } else {
        crc >>= 1;
      }
    }
  }
  return crc;
}

/**
 * Build a Modbus RTU Read Holding Registers (FC 03) frame
 */
export function buildReadHoldingRegisters(deviceAddress: number, startAddress: number, quantity: number): Uint8Array {
  const frame = new Uint8Array(8);
  frame[0] = deviceAddress;
  frame[1] = 0x03; // Function Code
  frame[2] = (startAddress >> 8) & 0xFF;
  frame[3] = startAddress & 0xFF;
  frame[4] = (quantity >> 8) & 0xFF;
  frame[5] = quantity & 0xFF;
  
  const crc = calculateCRC(frame, 6);
  frame[6] = crc & 0xFF; // CRC Low
  frame[7] = (crc >> 8) & 0xFF; // CRC High
  
  return frame;
}

/**
 * Build a Modbus RTU Write Single Register (FC 06) frame
 */
export function buildWriteSingleRegister(deviceAddress: number, registerAddress: number, value: number): Uint8Array {
  const frame = new Uint8Array(8);
  frame[0] = deviceAddress;
  frame[1] = 0x06; // Function Code
  frame[2] = (registerAddress >> 8) & 0xFF;
  frame[3] = registerAddress & 0xFF;
  frame[4] = (value >> 8) & 0xFF;
  frame[5] = value & 0xFF;
  
  const crc = calculateCRC(frame, 6);
  frame[6] = crc & 0xFF; // CRC Low
  frame[7] = (crc >> 8) & 0xFF; // CRC High
  
  return frame;
}

/**
 * Build a Modbus RTU Write Multiple Registers (FC 16) frame
 */
export function buildWriteMultipleRegisters(deviceAddress: number, startAddress: number, values: number[]): Uint8Array {
  const quantity = values.length;
  const byteCount = quantity * 2;
  const frame = new Uint8Array(9 + byteCount);
  
  frame[0] = deviceAddress;
  frame[1] = 0x10; // Function Code (16)
  frame[2] = (startAddress >> 8) & 0xFF;
  frame[3] = startAddress & 0xFF;
  frame[4] = (quantity >> 8) & 0xFF;
  frame[5] = quantity & 0xFF;
  frame[6] = byteCount;
  
  for (let i = 0; i < quantity; i++) {
    frame[7 + i * 2] = (values[i]! >> 8) & 0xFF;
    frame[8 + i * 2] = values[i]! & 0xFF;
  }
  
  const crc = calculateCRC(frame, 7 + byteCount);
  frame[7 + byteCount] = crc & 0xFF; // CRC Low
  frame[8 + byteCount] = (crc >> 8) & 0xFF; // CRC High
  
  return frame;
}

/**
 * Build a custom 8-byte Modbus-style command frame
 * Used for legacy vendor-specific function codes in the VB6 tool.
 */
export function buildCustomFunctionWrite(
  deviceAddress: number,
  functionCode: number,
  registerAddress: number,
  value: number,
): Uint8Array {
  const frame = new Uint8Array(8);
  frame[0] = deviceAddress & 0xFF;
  frame[1] = functionCode & 0xFF;
  frame[2] = (registerAddress >> 8) & 0xFF;
  frame[3] = registerAddress & 0xFF;
  frame[4] = (value >> 8) & 0xFF;
  frame[5] = value & 0xFF;

  const crc = calculateCRC(frame, 6);
  frame[6] = crc & 0xFF;
  frame[7] = (crc >> 8) & 0xFF;
  return frame;
}

export interface ModbusResponse {
  deviceAddress: number;
  functionCode: number;
  data: Uint8Array;
  isError: boolean;
  errorCode?: number;
}

/**
 * Parse and validate a Modbus RTU response frame
 * Returns null if frame is incomplete or invalid
 */
export function parseResponse(frame: Uint8Array): ModbusResponse | null {
  if (frame.length < 4) return null; // Minimum frame size (addr + fc + crc)
  
  const deviceAddress = frame[0]!;
  const functionCode = frame[1]!;
  const isError = (functionCode & 0x80) !== 0;
  
  let expectedLength = 0;
  if (isError) {
    expectedLength = 5; // addr(1) + fc(1) + err(1) + crc(2)
  } else if (functionCode === 0x03) {
    const byteCount = frame[2]!;
    expectedLength = 5 + byteCount; // addr(1) + fc(1) + bc(1) + data(N) + crc(2)
  } else {
    // Most write and vendor-specific commands in this tooling are fixed 8-byte responses.
    expectedLength = 8; // addr(1) + fc(1) + payload(4) + crc(2)
  }
  
  if (frame.length < expectedLength) return null; // Incomplete frame
  
  // Validate CRC
  const actualFrame = frame.slice(0, expectedLength);
  const calculatedCrc = calculateCRC(actualFrame, expectedLength - 2);
  const receivedCrc = actualFrame[expectedLength - 2]! | (actualFrame[expectedLength - 1]! << 8);
  
  if (calculatedCrc !== receivedCrc) {
    console.warn(`CRC mismatch. Expected ${calculatedCrc.toString(16)}, got ${receivedCrc.toString(16)}`);
    return null;
  }
  
  if (isError) {
    return {
      deviceAddress,
      functionCode: functionCode & 0x7F,
      data: new Uint8Array(0),
      isError: true,
      errorCode: frame[2]!
    };
  }
  
  let data: Uint8Array;
  if (functionCode === 0x03) {
    const dataLen = frame[2]!;
    data = frame.slice(3, 3 + dataLen);
  } else {
    data = frame.slice(2, 6);
  }
  
  return {
    deviceAddress,
    functionCode,
    data,
    isError: false
  };
}
