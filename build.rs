use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn main() {
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    println!("cargo:rerun-if-changed=frontend/dist/index.html");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("index.html.gz");

    let html_path = Path::new("frontend/dist/index.html");
    let content = if html_path.exists() {
        fs::read(html_path).unwrap()
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
