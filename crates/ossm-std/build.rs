use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let html_path = Path::new(&manifest_dir).join("../../web/apps/webui-std/dist/index.html");
    println!("cargo:rerun-if-changed={}", html_path.display());
    if let Some(dist) = html_path.parent() {
        println!("cargo:rerun-if-changed={}", dist.display());
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("index.html");
    let content = if html_path.exists() {
        fs::read(&html_path).unwrap()
    } else {
        println!("cargo:warning=web/apps/webui-std/dist/index.html missing; embedding placeholder");
        b"<!DOCTYPE html><html><body><h1>webui-std not built</h1><p>Run npm run build -w webui-std in web/</p></body></html>".to_vec()
    };
    fs::write(&dest, content).unwrap();
}
