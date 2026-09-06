//! AIM30 Modbus RTU frame build/parse and homing (caller supplies `exchange`).

use std::future::Future;

use ossm_core::modbus::aim30;
use rmodbus::{client::ModbusRequest, ModbusProto};

pub const DEFAULT_SLAVE: u8 = 1;
pub const STREAM_TIMEOUT_MS: u32 = 80;
pub const GENERIC_TIMEOUT_MS: u32 = 200;

pub fn expected_response_len(req: &[u8]) -> usize {
    if req.len() >= 6 {
        match req[1] {
            0x03 | 0x04 => {
                let qty = u16::from_be_bytes([req[4], req[5]]) as usize;
                5 + qty * 2
            }
            0x06 | 0x10 => 8,
            _ => 8,
        }
    } else {
        8
    }
}

pub fn parse_fc03_u16(frame: &[u8]) -> Option<Vec<u16>> {
    if frame.len() < 5 || frame[1] != 0x03 {
        return None;
    }
    let n = frame[2] as usize;
    if !n.is_multiple_of(2) || frame.len() < 3 + n + 2 {
        return None;
    }
    let mut out = Vec::with_capacity(n / 2);
    let data = &frame[3..3 + n];
    for chunk in data.chunks_exact(2) {
        out.push(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    Some(out)
}

fn encode_set_holdings(slave: u8, addr: u16, values: &[u16]) -> Result<Vec<u8>, String> {
    let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
    let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
    let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
    request
        .generate_set_holdings_bulk(addr, values, &mut frame_buf)
        .map_err(|e| format!("modbus encode: {e:?}"))?;
    Ok(frame_buf.as_slice().to_vec())
}

fn encode_get_holdings(slave: u8, addr: u16, count: u16) -> Result<Vec<u8>, String> {
    let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
    let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
    let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
    request
        .generate_get_holdings(addr, count, &mut frame_buf)
        .map_err(|e| format!("modbus encode: {e:?}"))?;
    Ok(frame_buf.as_slice().to_vec())
}

pub fn position_write_frame(slave: u8, position: f32) -> Result<Vec<u8>, String> {
    let counts = aim30::write_counts_for_radians(position);
    let regs = aim30::pack_position_i32(counts);
    encode_set_holdings(slave, aim30::REG_POSITION, &regs)
}

pub async fn write_holding<Ex, Fut>(
    exchange: &mut Ex,
    slave: u8,
    addr: u16,
    value: u16,
    timeout_ms: u32,
) -> Result<(), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
    let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
    let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
    request
        .generate_set_holding(addr, value, &mut frame_buf)
        .map_err(|e| format!("modbus encode: {e:?}"))?;
    let tx = frame_buf.as_slice().to_vec();
    let rsp = exchange(tx, timeout_ms).await?;
    request
        .parse_ok(&rsp)
        .map_err(|e| format!("modbus parse: {e:?}"))
}

pub async fn write_holdings<Ex, Fut>(
    exchange: &mut Ex,
    slave: u8,
    addr: u16,
    values: &[u16],
    timeout_ms: u32,
) -> Result<(), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
    let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
    let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
    request
        .generate_set_holdings_bulk(addr, values, &mut frame_buf)
        .map_err(|e| format!("modbus encode: {e:?}"))?;
    let tx = frame_buf.as_slice().to_vec();
    let rsp = exchange(tx, timeout_ms).await?;
    request
        .parse_ok(&rsp)
        .map_err(|e| format!("modbus parse: {e:?}"))
}

pub async fn read_holdings<Ex, Fut>(
    exchange: &mut Ex,
    slave: u8,
    addr: u16,
    count: u16,
    timeout_ms: u32,
) -> Result<Vec<u16>, String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let tx = encode_get_holdings(slave, addr, count)?;
    let rsp = exchange(tx, timeout_ms).await?;
    parse_fc03_u16(&rsp).ok_or_else(|| "modbus parse: fc03".into())
}

pub async fn write_position_radians<Ex, Fut>(
    exchange: &mut Ex,
    slave: u8,
    position: f32,
    timeout_ms: u32,
    strict: bool,
) -> Result<(), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let counts = aim30::write_counts_for_radians(position);
    let regs = aim30::pack_position_i32(counts);
    match write_holdings(exchange, slave, aim30::REG_POSITION, &regs, timeout_ms).await {
        Ok(()) => Ok(()),
        Err(e) if !strict && e.starts_with("no CRC-valid response") => Ok(()),
        Err(e) => Err(e),
    }
}

pub async fn read_position_radians<Ex, Fut>(exchange: &mut Ex, slave: u8) -> Result<f32, String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let regs = read_holdings(exchange, slave, aim30::REG_POSITION, 2, GENERIC_TIMEOUT_MS).await?;
    if regs.len() < 2 {
        return Err("short position read".into());
    }
    let counts = aim30::unpack_position_i32([regs[0], regs[1]]);
    Ok(aim30::counts_to_radians(counts))
}

pub async fn enable_modbus<Ex, Fut>(exchange: &mut Ex, slave: u8) -> Result<(), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    write_holding(
        exchange,
        slave,
        aim30::REG_ENABLE,
        aim30::ENABLE_MODBUS,
        GENERIC_TIMEOUT_MS,
    )
    .await
}

pub async fn apply_run_gains<Ex, Fut>(exchange: &mut Ex, slave: u8) -> Result<(), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    write_holding(
        exchange,
        slave,
        aim30::REG_POWER,
        aim30::encode_max_power(aim30::POWER_RUN),
        GENERIC_TIMEOUT_MS,
    )
    .await?;
    write_holding(
        exchange,
        slave,
        aim30::REG_ACCEL,
        aim30::encode_acceleration(aim30::ACCEL_RUN),
        GENERIC_TIMEOUT_MS,
    )
    .await?;
    write_holding(
        exchange,
        slave,
        aim30::REG_POS_RING,
        aim30::RING_RATIO_RUN,
        GENERIC_TIMEOUT_MS,
    )
    .await?;
    write_holding(
        exchange,
        slave,
        aim30::REG_SPEED_RING,
        aim30::RING_RATIO_RUN,
        GENERIC_TIMEOUT_MS,
    )
    .await
}

async fn wait_stable_position<Ex, Fut, Sl, SlFut>(
    exchange: &mut Ex,
    sleep_ms: &mut Sl,
    slave: u8,
    timeout_ms: u32,
) -> Result<f32, String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
    Sl: FnMut(u32) -> SlFut,
    SlFut: Future<Output = ()>,
{
    let mut elapsed = 0u32;
    let mut position = read_position_radians(exchange, slave).await?;
    sleep_ms(100).await;
    elapsed += 100;
    while elapsed < timeout_ms {
        let new_position = read_position_radians(exchange, slave).await?;
        if (new_position - position).abs() < 0.002 {
            return Ok(new_position);
        }
        position = new_position;
        sleep_ms(100).await;
        elapsed += 100;
    }
    Err("stable position timeout".into())
}

/// Travel homing using core AIM30 literals. `sleep_ms` is a no-op in host tests.
pub async fn homing<Ex, Fut, Sl, SlFut>(
    exchange: &mut Ex,
    sleep_ms: &mut Sl,
    slave: u8,
) -> Result<(f32, f32), String>
where
    Ex: FnMut(Vec<u8>, u32) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
    Sl: FnMut(u32) -> SlFut,
    SlFut: Future<Output = ()>,
{
    log::info!("Homing motor");
    let mut power_ok = false;
    for attempt in 1..=8 {
        match write_holding(
            exchange,
            slave,
            aim30::REG_POWER,
            aim30::encode_max_power(aim30::POWER_HOMING),
            GENERIC_TIMEOUT_MS,
        )
        .await
        {
            Ok(()) => {
                power_ok = true;
                break;
            }
            Err(e) => {
                log::warn!("set_max_power POWER_HOMING ({attempt}/8): {e}");
                sleep_ms(200).await;
            }
        }
    }
    if !power_ok {
        return Err("set_max_power POWER_HOMING failed after retries".into());
    }
    let want = aim30::encode_max_power(aim30::POWER_HOMING);
    let regs = read_holdings(exchange, slave, aim30::REG_POWER, 1, GENERIC_TIMEOUT_MS).await?;
    let got = regs.first().copied();
    if got != Some(want) {
        return Err(format!(
            "REG_POWER readback {got:?} want {want} (POWER_HOMING)"
        ));
    }
    write_holding(
        exchange,
        slave,
        aim30::REG_ACCEL,
        aim30::encode_acceleration(aim30::ACCEL_HOMING),
        GENERIC_TIMEOUT_MS,
    )
    .await?;
    let zero = aim30::pack_position_i32(0);
    write_holdings(
        exchange,
        slave,
        aim30::REG_POSITION,
        &zero,
        GENERIC_TIMEOUT_MS,
    )
    .await?;

    write_position_radians(exchange, slave, -100.0, GENERIC_TIMEOUT_MS, true).await?;
    sleep_ms(5000).await;
    let pos_min = wait_stable_position(exchange, sleep_ms, slave, 5000).await? + 0.1;

    write_position_radians(exchange, slave, 100.0, GENERIC_TIMEOUT_MS, true).await?;
    sleep_ms(5000).await;
    let pos_max = wait_stable_position(exchange, sleep_ms, slave, 5000).await? - 0.1;

    write_position_radians(
        exchange,
        slave,
        (pos_min + pos_max) / 2.0,
        GENERIC_TIMEOUT_MS,
        true,
    )
    .await?;
    sleep_ms(5000).await;
    wait_stable_position(exchange, sleep_ms, slave, 5000).await?;
    Ok((pos_min, pos_max))
}
