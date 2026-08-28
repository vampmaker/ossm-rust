use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::state::StreamWaypoint;

#[derive(Deserialize)]
pub struct WsMessage {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(alias = "method")]
    pub cmd: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl WsMessage {
    pub fn parse_params<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        if let Some(ref params) = self.params {
            if !params.is_null() && (params.is_object() || params.is_array()) {
                return serde_json::from_value(params.clone());
            }
        }
        if let Some(config_val) = self.extra.get("config") {
            if !config_val.is_null() && config_val.is_object() {
                return serde_json::from_value(config_val.clone());
            }
        }
        serde_json::from_value(self.extra.clone())
    }
}

#[derive(Deserialize)]
pub struct WaypointsObject {
    pub waypoints: Vec<StreamWaypoint>,
    #[serde(
        default,
        rename = "reset-timestamp",
        alias = "reset_timestamp",
        alias = "reset"
    )]
    pub reset_timestamp: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum WaypointsInput {
    List(Vec<StreamWaypoint>),
    Object(WaypointsObject),
    Single(StreamWaypoint),
}

impl WaypointsInput {
    pub fn into_parts(self) -> (Vec<StreamWaypoint>, bool) {
        match self {
            WaypointsInput::List(list) => (list, false),
            WaypointsInput::Object(obj) => (obj.waypoints, obj.reset_timestamp.unwrap_or(false)),
            WaypointsInput::Single(wp) => (alloc::vec![wp], false),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct SubscribeParams {
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct PausedControl {
    pub paused: Option<bool>,
    pub position: Option<f32>,
    /// Alias for [`Self::position`] — some clients send the motor-config field name.
    #[serde(default)]
    pub paused_position: Option<f32>,
    pub adjust: Option<f32>,
}
