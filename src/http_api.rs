use esp_idf_svc::http::server::{EspHttpServer, Method};
use serde::{Deserialize, Serialize};
use crate::motion::MotorControllerConfig;
use esp_idf_svc::io::{Read, Write};
use crate::context::AppContext;

#[derive(Serialize, Deserialize)]
pub struct PausedControl {
    pub paused: Option<bool>,              // Set paused state
    pub position: Option<f32>,             // Set absolute position
    pub adjust: Option<f32>,               // Adjust position relatively (positive or negative)
}

const APP_HTML: &[u8] = include_bytes!("../frontend/dist/index.html");

struct IoCompat<'a, T> {
    inner: &'a mut T,
    read_count: usize,
    limit: usize,
}

impl<'a, T> IoCompat<'a, T> {
    fn new(inner: &'a mut T, limit: usize) -> Self {
        Self {
            inner,
            read_count: 0,
            limit,
        }
    }
}

impl<'a, T> std::io::Read for IoCompat<'a, T>
where
    T: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.read_count >= self.limit {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "Request too large"));
        }
        let max_len = buf.len().min(self.limit - self.read_count);
        let n = self.inner.read(&mut buf[..max_len]).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e)))?;
        self.read_count += n;
        Ok(n)
    }
}

pub fn register_handlers<'a>(
    server: &mut EspHttpServer<'a>,
    app_context: AppContext,
) {
    // CORS preflight handlers
    {
        server.fn_handler::<anyhow::Error, _>("/config", Method::Options, |req| {
            req.into_response(200, Some("OK"), &[
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
                ("Access-Control-Allow-Headers", "*"),
            ])?
                .write_all(&[])?;
            Ok(())
        }).unwrap();
        server.fn_handler::<anyhow::Error, _>("/paused", Method::Options, |req| {
            req.into_response(200, Some("OK"), &[
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Methods", "POST, OPTIONS"),
                ("Access-Control-Allow-Headers", "*"),
            ])?
                .write_all(&[])?;
            Ok(())
        }).unwrap();
        server.fn_handler::<anyhow::Error, _>("/state", Method::Options, |req| {
            req.into_response(200, Some("OK"), &[
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Methods", "GET, OPTIONS"),
                ("Access-Control-Allow-Headers", "*"),
            ])?
                .write_all(&[])?;
            Ok(())
        }).unwrap();
    }

    {
        let controller = app_context.motor_controller.clone();
        server.fn_handler::<anyhow::Error, _>("/config", Method::Get, move |req| {
            let mut mc_opt = controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                let config = mc.get_config();
                let json = serde_json::to_string(&config).unwrap();
                req.into_response(200, Some("OK"), &[("Access-Control-Allow-Origin", "*")])?
                    .write_all(json.as_bytes())?;
            } else {
                req.into_response(503, Some("Service Unavailable"), &[("Access-Control-Allow-Origin", "*")])?
                    .write_all("Motor controller not initialized".as_bytes())?;
            }
            Ok(())
        }).unwrap();
    }

    {
        let controller = app_context.motor_controller.clone();
        server.fn_handler::<anyhow::Error, _>("/config", Method::Post, move |mut req| {
            match serde_json::from_reader::<_, MotorControllerConfig>(IoCompat::new(&mut req, 8192)) {
                Ok(config) => {
                    let json = serde_json::to_string(&config).unwrap();
                    let mut mc_opt = controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        mc.set_config(config).unwrap();
                        req.into_response(200, Some("OK"), &[("Access-Control-Allow-Origin", "*")])?
                            .write_all(json.as_bytes())?;
                    } else {
                        req.into_response(503, Some("Service Unavailable"), &[("Access-Control-Allow-Origin", "*")])?
                            .write_all("Motor controller not initialized".as_bytes())?;
                    }
                }
                Err(e) => {
                    log::error!("Failed to parse config: {}", e);
                    req.into_response(400, None, &[("Access-Control-Allow-Origin", "*")])?
                        .write_all("Bad Request".as_bytes())?;
                }
            }
            Ok(())
        }).unwrap();
    }

    {
        let controller = app_context.motor_controller.clone();
        server.fn_handler::<anyhow::Error, _>("/paused", Method::Post, move |mut req| {
            match serde_json::from_reader::<_, PausedControl>(IoCompat::new(&mut req, 8192)) {
                Ok(control) => {
                    let mut mc_opt = controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        let mut config = mc.get_config();

                        if let Some(paused) = control.paused {
                            config.paused = paused;
                        }
                        if let Some(position) = control.position {
                            config.paused_position = position.max(0.0).min(1.0);
                        }
                        if let Some(adjust) = control.adjust {
                            config.paused_position = (config.paused_position + adjust).max(0.0).min(1.0);
                        }

                        mc.set_config(config.clone()).unwrap();
                        let json = serde_json::to_string(&config).unwrap();
                        req.into_response(200, Some("OK"), &[("Access-Control-Allow-Origin", "*")])?
                            .write_all(json.as_bytes())?;
                    } else {
                        req.into_response(503, Some("Service Unavailable"), &[("Access-Control-Allow-Origin", "*")])?
                            .write_all("Motor controller not initialized".as_bytes())?;
                    }
                }
                Err(e) => {
                    log::error!("Failed to parse paused control: {}", e);
                    req.into_response(400, None, &[("Access-Control-Allow-Origin", "*")])?
                        .write_all("Bad Request".as_bytes())?;
                }
            }
            Ok(())
        }).unwrap();
    }

    {
        let controller = app_context.motor_controller.clone();
        server.fn_handler::<anyhow::Error, _>("/state", Method::Get, move |req| {
            let mut mc_opt = controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                let state = mc.get_current_state();
                let json = serde_json::to_string(&state).unwrap();
                req.into_response(200, Some("OK"), &[("Access-Control-Allow-Origin", "*")])?
                    .write_all(json.as_bytes())?;
            } else {
                req.into_response(503, Some("Service Unavailable"), &[("Access-Control-Allow-Origin", "*")])?
                    .write_all("Motor controller not initialized".as_bytes())?;
            }
            Ok(())
        }).unwrap();
    }

    {
        server.fn_handler::<anyhow::Error, _>("/", Method::Get, move |req| {
            req.into_response(200, Some("OK"), &[("Access-Control-Allow-Origin", "*"), ("Content-Type", "text/html")])?
                .write_all(APP_HTML)?;
            Ok(())
        }).unwrap();
    }
}
