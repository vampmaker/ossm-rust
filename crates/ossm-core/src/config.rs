use core::fmt;
use core::str::FromStr;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const SPLINE_POINTS_CAP: usize = 16;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaveFunc {
    #[default]
    Sine,
    Thrust,
    Spline,
}

impl WaveFunc {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Thrust => "thrust",
            Self::Spline => "spline",
        }
    }
}

impl fmt::Display for WaveFunc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WaveFunc {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sine" => Ok(Self::Sine),
            "thrust" => Ok(Self::Thrust),
            "spline" => Ok(Self::Spline),
            _ => Err(()),
        }
    }
}

/// JSON array of used points; stored in a fixed buffer so config is `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplinePoints {
    buf: [f32; SPLINE_POINTS_CAP],
    len: u8,
}

impl SplinePoints {
    pub fn from_slice(src: &[f32]) -> Self {
        let mut buf = [0.0; SPLINE_POINTS_CAP];
        let n = src.len().min(SPLINE_POINTS_CAP);
        if n < 2 {
            buf[0] = 0.0;
            buf[1] = 1.0;
            return Self { buf, len: 2 };
        }
        buf[..n].copy_from_slice(&src[..n]);
        Self { buf, len: n as u8 }
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.buf[..self.len as usize]
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for SplinePoints {
    fn default() -> Self {
        Self::from_slice(&[0.0, 1.0])
    }
}

impl Serialize for SplinePoints {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SplinePoints {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = SplinePoints;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON array of spline points")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut tmp = [0.0f32; SPLINE_POINTS_CAP];
                let mut n = 0usize;
                while let Some(v) = seq.next_element::<f32>()? {
                    if n < SPLINE_POINTS_CAP {
                        tmp[n] = v;
                        n += 1;
                    }
                }
                Ok(SplinePoints::from_slice(&tmp[..n]))
            }
        }
        deserializer.deserialize_seq(V)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MotorControllerConfig {
    #[serde(default)]
    pub version: u32,
    pub bpm: f32,
    pub depth: f32,
    pub depth_top: bool,
    pub reversed: bool,
    pub wave_func: WaveFunc,
    pub sharpness: f32,
    #[serde(default)]
    pub spline_points: SplinePoints,
    pub paused: bool,
    pub paused_position: f32,
    #[serde(default)]
    pub streaming: bool,
}

impl MotorControllerConfig {
    pub fn copy_from(&mut self, src: &MotorControllerConfig) {
        *self = *src;
    }

    /// Persistent fields (NVS / JSON / IndexedDB). Ignores pause pose.
    pub fn persistent_motor_changed(&self, other: &Self) -> bool {
        self.bpm != other.bpm
            || self.depth != other.depth
            || self.depth_top != other.depth_top
            || self.reversed != other.reversed
            || self.wave_func != other.wave_func
            || self.sharpness != other.sharpness
            || self.spline_points != other.spline_points
            || self.streaming != other.streaming
    }
}

impl Default for MotorControllerConfig {
    fn default() -> Self {
        Self {
            version: 0,
            bpm: 36.0,
            depth: 1.0,
            depth_top: false,
            reversed: false,
            wave_func: WaveFunc::Sine,
            sharpness: 0.3,
            spline_points: SplinePoints::default(),
            paused: true,
            paused_position: 0.0,
            streaming: false,
        }
    }
}
