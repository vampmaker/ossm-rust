use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Write as _;

use crate::config::MotorControllerConfig;
use crate::engine::Engine;
use crate::error::CoreError;

pub const PATHS_CATALOG: &str = "\
Config paths (get <path> / set <path> <value>):

motor — full MotorControllerConfig JSON
  motor.bpm                                    > 0
  motor.depth                                  0.01..1
  motor.depth_top / reversed / paused / streaming   true|false
  motor.wave_func                              sine|thrust|spline
  motor.sharpness                              0.01..0.99
  motor.paused_position                        0..1
  motor.spline_points                          space-separated floats
  set motor {\"bpm\":36,...}                   bulk JSON (read-only: version)

Actions: reset, get-state, get-status, reset-timestamp,
         set-waypoints <json>, append-waypoints <json>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    InvalidPath,
    UnknownSection,
    UnknownKey,
    InvalidValue,
    ReadOnly,
    ScalarRequired,
    BulkJsonRequired,
    StaleVersion,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => write!(f, "Invalid path"),
            Self::UnknownSection => write!(f, "Unknown section (try: motor)"),
            Self::UnknownKey => write!(f, "Unknown key"),
            Self::InvalidValue => write!(f, "Invalid value"),
            Self::ReadOnly => write!(f, "read-only"),
            Self::ScalarRequired => write!(f, "Use scalar path"),
            Self::BulkJsonRequired => write!(f, "Bulk motor set requires JSON"),
            Self::StaleVersion => write!(f, "Stale causal version"),
        }
    }
}

pub fn parse_path(path: &str) -> Option<(&str, Option<&str>)> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    match path.split_once('.') {
        Some((section, key)) if !section.is_empty() && !key.is_empty() => {
            Some((section, Some(key)))
        }
        _ => Some((path, None)),
    }
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

pub fn get(engine: &Engine, path: &str) -> Result<String, PathError> {
    let (section, key) = parse_path(path).ok_or(PathError::InvalidPath)?;
    match section {
        "motor" => match key {
            None => serde_json::to_string(&engine.snapshot().config)
                .map_err(|_| PathError::InvalidValue),
            Some(k) => motor_field(&engine.snapshot().config, k),
        },
        _ => Err(PathError::UnknownSection),
    }
}

pub fn set(engine: &mut Engine, path: &str, value: &str) -> Result<String, PathError> {
    let (section, key) = parse_path(path).ok_or(PathError::InvalidPath)?;
    match section {
        "motor" => {
            let Some(key) = key else {
                return Err(PathError::BulkJsonRequired);
            };
            let mut cfg = engine.snapshot().config.clone();
            apply_motor_field(&mut cfg, key, value)?;
            match engine.try_set_config(cfg) {
                Ok(_) => Ok(String::from(value)),
                Err(CoreError::StaleVersion) => Err(PathError::StaleVersion),
                Err(_) => Err(PathError::InvalidValue),
            }
        }
        _ => Err(PathError::UnknownSection),
    }
}

pub fn motor_field(cfg: &MotorControllerConfig, key: &str) -> Result<String, PathError> {
    match key {
        "version" => Ok(format!("{}", cfg.version)),
        "bpm" => Ok(format!("{}", cfg.bpm)),
        "depth" => Ok(format!("{}", cfg.depth)),
        "depth_top" => Ok(bool_str(cfg.depth_top)),
        "reversed" => Ok(bool_str(cfg.reversed)),
        "paused" => Ok(bool_str(cfg.paused)),
        "streaming" => Ok(bool_str(cfg.streaming)),
        "wave_func" => Ok(cfg.wave_func.clone()),
        "sharpness" => Ok(format!("{}", cfg.sharpness)),
        "paused_position" => Ok(format!("{}", cfg.paused_position)),
        "spline_points" => Ok(format_spline(&cfg.spline_points)),
        _ => Err(PathError::UnknownKey),
    }
}

pub fn apply_motor_field(
    cfg: &mut MotorControllerConfig,
    key: &str,
    value: &str,
) -> Result<(), PathError> {
    match key {
        "version" => Err(PathError::ReadOnly),
        "bpm" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if v <= 0.0 {
                return Err(PathError::InvalidValue);
            }
            cfg.bpm = v;
            Ok(())
        }
        "depth" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.01..=1.0).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.depth = v;
            Ok(())
        }
        "depth_top" => {
            cfg.depth_top = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "reversed" => {
            cfg.reversed = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "paused" => {
            cfg.paused = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "streaming" => {
            cfg.streaming = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "wave_func" => {
            if !matches!(value, "sine" | "thrust" | "spline") {
                return Err(PathError::InvalidValue);
            }
            cfg.wave_func = String::from(value);
            Ok(())
        }
        "sharpness" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.01..=0.99).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.sharpness = v;
            Ok(())
        }
        "paused_position" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.0..=1.0).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.paused_position = v;
            Ok(())
        }
        "spline_points" => {
            let parsed: Result<Vec<f32>, _> =
                value.split_whitespace().map(|s| s.parse::<f32>()).collect();
            cfg.spline_points = parsed.map_err(|_| PathError::InvalidValue)?;
            Ok(())
        }
        _ => Err(PathError::UnknownKey),
    }
}

fn bool_str(v: bool) -> String {
    String::from(if v { "true" } else { "false" })
}

fn format_spline(pts: &[f32]) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{}", p);
    }
    s
}
