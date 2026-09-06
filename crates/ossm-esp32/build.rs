use flate2::write::GzEncoder;
use flate2::Compression;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    if env::var("CARGO_FEATURE_ESP32S3").is_ok() {
        let ld_dir = Path::new(&manifest_dir).join("ld/esp32s3");
        println!(
            "cargo:rustc-link-arg=-T{}",
            ld_dir.join("linkall.x").display()
        );
        println!("cargo:rustc-link-search={}", ld_dir.display());
        println!("cargo:rerun-if-changed={}", ld_dir.display());
    } else {
        println!("cargo:rustc-link-arg=-Tlinkall.x");
    }

    let html_path = Path::new(&manifest_dir).join("../../web/apps/webui-esp32/dist/index.html");
    println!("cargo:rerun-if-changed={}", html_path.display());

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("index.html.gz");
    let content = if html_path.exists() {
        fs::read(&html_path).unwrap()
    } else {
        b"<html><body><h1>Frontend not built</h1></body></html>".to_vec()
    };

    let mut encoder = GzEncoder::new(File::create(&dest_path).unwrap(), Compression::best());
    encoder.write_all(&content).unwrap();
    encoder.finish().unwrap();

    if let Some(parent) = html_path.parent() {
        if parent.exists() {
            let dist_gz = parent.join("index.html.gz");
            if let Ok(file) = File::create(&dist_gz) {
                let mut encoder = GzEncoder::new(file, Compression::best());
                let _ = encoder.write_all(&content);
                let _ = encoder.finish();
            }
        }
    }
}
