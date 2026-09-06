//! Stdin REPL matching firmware `get`/`set` path CLI (`ossm_core::paths`).

use std::sync::Arc;

use ossm_core::paths::{self, PathError};
use ossm_core::{Command, MotionCommand, WaypointsInput};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

use crate::engine_task::{EngineHandle, EngineMsg};

#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    Get(String),
    Set { path: String, value: String },
    Paths,
    Help,
    Reset,
    GetState,
    GetStatus,
    ResetTimestamp,
    SetWaypoints(String),
    AppendWaypoints(String),
    Quit,
}

pub fn parse_line(raw: &str) -> Result<Option<Line>, String> {
    let line = raw.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let (cmd, rest) = match line.split_once(char::is_whitespace) {
        Some((c, r)) => (c, r.trim()),
        None => (line, ""),
    };
    let cmd = cmd.to_ascii_lowercase();
    match cmd.as_str() {
        "help" | "?" => Ok(Some(Line::Help)),
        "paths" => Ok(Some(Line::Paths)),
        "reset" => Ok(Some(Line::Reset)),
        "quit" | "exit" => Ok(Some(Line::Quit)),
        "get-state" => Ok(Some(Line::GetState)),
        "get-status" => Ok(Some(Line::GetStatus)),
        "reset-timestamp" => Ok(Some(Line::ResetTimestamp)),
        "get" => {
            if rest.is_empty() {
                Err("Usage: get <path>".into())
            } else {
                Ok(Some(Line::Get(rest.to_string())))
            }
        }
        "set" => {
            let Some((path, value)) = rest.split_once(char::is_whitespace) else {
                return Err("Usage: set <path> <value>".into());
            };
            let value = strip_quotes(value.trim());
            if path.is_empty() || value.is_empty() {
                return Err("Usage: set <path> <value>".into());
            }
            Ok(Some(Line::Set {
                path: path.to_string(),
                value,
            }))
        }
        "set-waypoints" => {
            if rest.is_empty() {
                Err("Usage: set-waypoints <json>".into())
            } else {
                Ok(Some(Line::SetWaypoints(rest.to_string())))
            }
        }
        "append-waypoints" => {
            if rest.is_empty() {
                Err("Usage: append-waypoints <json>".into())
            } else {
                Ok(Some(Line::AppendWaypoints(rest.to_string())))
            }
        }
        _ => Err(format!("Unknown command: {cmd} (try: help)")),
    }
}

fn strip_quotes(value: &str) -> String {
    let v = value.trim();
    if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
    {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

pub async fn run(engine: EngineHandle, restart: Arc<dyn Fn() + Send + Sync>) {
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if tty {
        let _ = stdout
            .write_all(b"ossm-std CLI - type help or paths\n")
            .await;
    }
    loop {
        if tty {
            let _ = stdout.write_all(b"ossm> ").await;
            let _ = stdout.flush().await;
        }
        line.clear();
        match stdin.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("stdin: {e}");
                break;
            }
        }
        match parse_line(&line) {
            Ok(None) => {}
            Ok(Some(cmd)) => {
                if matches!(cmd, Line::Quit) {
                    break;
                }
                if matches!(cmd, Line::Reset) {
                    restart();
                    break;
                }
                if let Err(e) = execute(&engine, cmd, &mut stdout).await {
                    let _ = stdout.write_all(format!("{e}\n").as_bytes()).await;
                }
            }
            Err(e) => {
                let _ = stdout.write_all(format!("{e}\n").as_bytes()).await;
            }
        }
    }
}

async fn execute(
    engine: &EngineHandle,
    cmd: Line,
    stdout: &mut tokio::io::Stdout,
) -> Result<(), String> {
    match cmd {
        Line::Help | Line::Paths => {
            writeln_out(stdout, paths::PATHS_CATALOG).await?;
            writeln_out(stdout, "quit / exit — leave REPL (HTTP keeps running)").await
        }
        Line::Get(path) => {
            let v = path_get(engine, &path).await?;
            let (section, key) = paths::parse_path(&path).unwrap_or((path.as_str(), None));
            if key.is_none() && section == "motor" {
                writeln_out(stdout, &v).await
            } else {
                writeln_out(stdout, &format!("{path}: {v}")).await
            }
        }
        Line::Set { path, value } => {
            let shown = path_set(engine, &path, &value).await?;
            writeln_out(stdout, &format!("{path} set to {shown}")).await
        }
        Line::GetState => {
            let snap = *engine.snap.borrow();
            let json = serde_json::to_string(&snap).map_err(|e| e.to_string())?;
            writeln_out(stdout, &json).await
        }
        Line::GetStatus => {
            let snap = *engine.snap.borrow();
            let dump = ossm_core::CliStatusDump {
                state: &snap,
                config: &snap.config,
            };
            let json = serde_json::to_string(&dump).map_err(|e| e.to_string())?;
            writeln_out(stdout, &json).await
        }
        Line::ResetTimestamp => {
            apply(engine, MotionCommand::ResetTimestamp.into()).await?;
            writeln_out(stdout, "ok").await
        }
        Line::SetWaypoints(json) => {
            let input: WaypointsInput =
                serde_json::from_str(&json).map_err(|_| "Invalid waypoints JSON".to_string())?;
            let (waypoints, reset_timestamp) = input.into_parts();
            apply(
                engine,
                MotionCommand::SetWaypoints {
                    waypoints,
                    reset_timestamp,
                }
                .into(),
            )
            .await?;
            writeln_out(stdout, "ok").await
        }
        Line::AppendWaypoints(json) => {
            let input: WaypointsInput =
                serde_json::from_str(&json).map_err(|_| "Invalid waypoints JSON".to_string())?;
            let (waypoints, _) = input.into_parts();
            apply(engine, MotionCommand::AppendWaypoints(waypoints).into()).await?;
            writeln_out(stdout, "ok").await
        }
        Line::Reset | Line::Quit => Ok(()),
    }
}

async fn writeln_out(stdout: &mut tokio::io::Stdout, s: &str) -> Result<(), String> {
    stdout
        .write_all(s.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    if !s.ends_with('\n') {
        stdout.write_all(b"\n").await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn format_path_err(path: &str, err: PathError) -> String {
    match err {
        PathError::UnknownKey => format!("Unknown key: {path}"),
        PathError::UnknownSection => err.to_string(),
        PathError::ReadOnly => "motor.version is read-only".into(),
        PathError::ScalarRequired => "Use scalar path, e.g. set motor.bpm 36".into(),
        PathError::BulkJsonRequired => "Bulk motor set requires JSON: set motor {...}".into(),
        PathError::InvalidValue => format!("Invalid value for {path}"),
        _ => format!("{path}: {err}"),
    }
}

async fn path_get(engine: &EngineHandle, path: &str) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    engine
        .tx
        .send(EngineMsg::PathGet {
            path: path.to_string(),
            reply: tx,
        })
        .await
        .map_err(|_| "engine task stopped".to_string())?;
    rx.await
        .map_err(|_| "engine task stopped".to_string())?
        .map_err(|e| format_path_err(path, e))
}

async fn path_set(engine: &EngineHandle, path: &str, value: &str) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    engine
        .tx
        .send(EngineMsg::PathSet {
            path: path.to_string(),
            value: value.to_string(),
            reply: tx,
        })
        .await
        .map_err(|_| "engine task stopped".to_string())?;
    rx.await
        .map_err(|_| "engine task stopped".to_string())?
        .map_err(|e| format_path_err(path, e))
}

async fn apply(engine: &EngineHandle, cmd: Command) -> Result<(), String> {
    engine
        .tx
        .send(EngineMsg::Apply(cmd))
        .await
        .map_err(|_| "engine task stopped".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_set() {
        assert_eq!(
            parse_line("get motor.bpm").unwrap(),
            Some(Line::Get("motor.bpm".into()))
        );
        assert_eq!(
            parse_line("set motor.bpm 36").unwrap(),
            Some(Line::Set {
                path: "motor.bpm".into(),
                value: "36".into(),
            })
        );
        assert_eq!(
            parse_line("set motor.wave_func sine").unwrap(),
            Some(Line::Set {
                path: "motor.wave_func".into(),
                value: "sine".into(),
            })
        );
        assert_eq!(
            parse_line("set motor.wave_func \"thrust\"").unwrap(),
            Some(Line::Set {
                path: "motor.wave_func".into(),
                value: "thrust".into(),
            })
        );
    }

    #[test]
    fn parse_actions() {
        assert_eq!(parse_line("paths").unwrap(), Some(Line::Paths));
        assert_eq!(parse_line("help").unwrap(), Some(Line::Help));
        assert_eq!(parse_line("get-state").unwrap(), Some(Line::GetState));
        assert_eq!(
            parse_line("reset-timestamp").unwrap(),
            Some(Line::ResetTimestamp)
        );
        assert_eq!(parse_line("quit").unwrap(), Some(Line::Quit));
        assert_eq!(parse_line("").unwrap(), None);
        assert!(parse_line("nope").is_err());
        assert_eq!(
            parse_line("set-waypoints [{\"ts\":1,\"pos\":0.5}]").unwrap(),
            Some(Line::SetWaypoints("[{\"ts\":1,\"pos\":0.5}]".into()))
        );
    }
}
