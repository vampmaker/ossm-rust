use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ossm_core::{MotorControllerConfig, NetworkConfiguration, PinConfiguration};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SavedConfig {
    #[serde(default)]
    pub pin: PinConfiguration,
    #[serde(default)]
    pub net: NetworkConfiguration,
    #[serde(default)]
    pub motor: MotorControllerConfig,
}

pub struct Persist {
    path: PathBuf,
}

impl Persist {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> SavedConfig {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => SavedConfig::default(),
        }
    }

    /// Write `path.tmp` in the same directory, `sync_all`, then `rename` over `path`.
    pub fn save(&self, cfg: &SavedConfig) -> io::Result<()> {
        atomic_write(&self.path, &serde_json::to_vec_pretty(cfg)?)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

pub fn persistent_motor_changed(a: &MotorControllerConfig, b: &MotorControllerConfig) -> bool {
    a.bpm != b.bpm
        || a.depth != b.depth
        || a.depth_top != b.depth_top
        || a.reversed != b.reversed
        || a.wave_func != b.wave_func
        || (a.sharpness - b.sharpness).abs() > f32::EPSILON
        || a.spline_points != b.spline_points
        || a.streaming != b.streaming
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ossm-std-persist-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rename_replaces_whole_file() {
        let dir = unique_dir();
        let path = dir.join("config.json");
        let persist = Persist::new(&path);
        let mut a = SavedConfig::default();
        a.motor.bpm = 10.0;
        persist.save(&a).unwrap();
        let mut b = SavedConfig::default();
        b.motor.bpm = 40.0;
        persist.save(&b).unwrap();
        let loaded = persist.load();
        assert_eq!(loaded.motor.bpm, 40.0);
        assert!(!tmp_path(&path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_before_rename_keeps_old_file() {
        let dir = unique_dir();
        let path = dir.join("config.json");
        let persist = Persist::new(&path);
        let mut old = SavedConfig::default();
        old.motor.bpm = 12.0;
        persist.save(&old).unwrap();

        let tmp = tmp_path(&path);
        fs::write(&tmp, b"{not valid json and truncated").unwrap();
        assert!(path.exists());
        let loaded = persist.load();
        assert_eq!(loaded.motor.bpm, 12.0);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("12"));
        let _ = fs::remove_dir_all(&dir);
    }
}
