#!/usr/bin/env -S uv run
"""Build web apps, ossm-wasm.html, ossm-std zigbuild binaries, and firmware images."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

from dotenv import load_dotenv

ROOT = Path(__file__).resolve().parent.parent
RELEASE = ROOT / "release"
WEB = ROOT / "web"
WASM_PKG = WEB / "apps" / "webui-wasm" / "src" / "pkg"
WASM_BINDGEN_VERSION = "0.2.127"
MACOSX_SDK_CACHE = ROOT / ".macos-sdk"
DEFAULT_MACOSX_SDK_URL = (
    "https://github.com/phracker/MacOSX-SDKs/releases/download/11.3/MacOSX11.3.sdk.tar.xz"
)

STD_TARGETS: list[tuple[str, str, str]] = [
    ("ossm-std-win-x64.exe", "x86_64-pc-windows-gnu", "ossm-std.exe"),
    ("ossm-std-linux-x64", "x86_64-unknown-linux-musl", "ossm-std"),
    ("ossm-std-linux-arm64", "aarch64-unknown-linux-musl", "ossm-std"),
    ("ossm-std-linux-armel", "arm-unknown-linux-musleabi", "ossm-std"),
    ("ossm-std-macos-arm64", "aarch64-apple-darwin", "ossm-std"),
]

ARTIFACTS: tuple[str, ...] = (
    "flasher.html",
    "motor-control.html",
    "ossm-wasm.html",
    "ossm-esp32c6.bin",
    "ossm-esp32s3.bin",
    *(dest for dest, _, _ in STD_TARGETS),
)


def run(cmd: list[str] | str, *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    printable = cmd if isinstance(cmd, str) else " ".join(cmd)
    print(f"==> {printable}", flush=True)
    subprocess.check_call(cmd, cwd=cwd or ROOT, env=env, shell=isinstance(cmd, str))


def rustup_installed_targets() -> set[str]:
    out = subprocess.check_output(
        ["rustup", "+stable", "target", "list", "--installed"],
        text=True,
    )
    return {line.strip() for line in out.splitlines() if line.strip()}


def rustup_add(*targets: str) -> None:
    installed = rustup_installed_targets()
    missing = [t for t in targets if t not in installed]
    skipped = [t for t in targets if t in installed]
    if skipped:
        print(f"==> rustup targets already installed: {' '.join(skipped)}", flush=True)
    if missing:
        run(["rustup", "+stable", "target", "add", *missing])


def ensure_wasm_bindgen() -> None:
    probe = subprocess.run(
        ["wasm-bindgen", "--version"],
        check=False,
        capture_output=True,
        text=True,
    )
    if probe.returncode == 0 and WASM_BINDGEN_VERSION in (probe.stdout + probe.stderr):
        return
    run(
        [
            "cargo",
            "+stable",
            "install",
            "wasm-bindgen-cli",
            "--version",
            WASM_BINDGEN_VERSION,
            "--locked",
        ]
    )


def firmware_bash(inner: str) -> None:
    export = Path.home() / "export-esp.sh"
    prefix = f"source '{export}' 2>/dev/null || true; " if export.exists() else ""
    run(["bash", "-lc", prefix + inner])


def prepare_release_dir() -> None:
    RELEASE.mkdir(parents=True, exist_ok=True)
    for name in ARTIFACTS:
        path = RELEASE / name
        if path.is_file() or path.is_symlink():
            path.unlink()


def zig_bin_dir() -> Path:
    import ziglang

    return Path(ziglang.__file__).resolve().parent


def zigbuild_env(*, sdkroot: Path | None = None) -> dict[str, str]:
    env = os.environ.copy()
    env["CARGO_ZIGBUILD_PYTHON_PATH"] = sys.executable
    # PyPI ziglang ships `zig` next to the package, but the console script is `python-zig`.
    env["PATH"] = str(zig_bin_dir()) + os.pathsep + env.get("PATH", "")
    if sdkroot is not None:
        env["SDKROOT"] = str(sdkroot)
        env["PKG_CONFIG_SYSROOT_DIR"] = str(sdkroot)
    return env


def macos_sdk_url() -> str:
    raw = os.environ.get("MACOSX_SDK_URL", "").strip().strip('"')
    return raw or DEFAULT_MACOSX_SDK_URL


def sdk_looks_valid(path: Path) -> bool:
    frameworks = path / "System" / "Library" / "Frameworks"
    return (frameworks / "IOKit.framework").exists() and (
        frameworks / "CoreFoundation.framework"
    ).exists()


def find_extracted_sdk(root: Path) -> Path | None:
    if not root.is_dir():
        return None
    if sdk_looks_valid(root):
        return root
    matches = sorted(p for p in root.glob("MacOSX*.sdk") if p.is_dir() and sdk_looks_valid(p))
    return matches[-1] if matches else None


def download_file(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".tmp")
    print(f"==> downloading {url}", flush=True)
    req = urllib.request.Request(url, headers={"User-Agent": "ossm-rust-release"})
    with urllib.request.urlopen(req, timeout=120) as src, tmp.open("wb") as out:
        shutil.copyfileobj(src, out)
    tmp.replace(dest)


def ensure_macos_sdk() -> Path:
    existing = os.environ.get("SDKROOT", "").strip().strip('"')
    if existing:
        path = Path(existing).expanduser().resolve()
        if sdk_looks_valid(path):
            print(f"==> using SDKROOT {path}")
            return path
        print(f"==> SDKROOT={path} is missing IOKit/CoreFoundation; downloading")

    cached = find_extracted_sdk(MACOSX_SDK_CACHE)
    if cached is not None:
        print(f"==> using cached macOS SDK {cached}")
        return cached

    url = macos_sdk_url()
    tar_path = MACOSX_SDK_CACHE / url.rsplit("/", 1)[-1]
    if not tar_path.is_file():
        download_file(url, tar_path)
    print(f"==> extracting {tar_path.name} -> {MACOSX_SDK_CACHE}")
    MACOSX_SDK_CACHE.mkdir(parents=True, exist_ok=True)
    with tarfile.open(tar_path) as tf:
        try:
            tf.extractall(MACOSX_SDK_CACHE, filter="data")
        except TypeError:
            tf.extractall(MACOSX_SDK_CACHE)
    cached = find_extracted_sdk(MACOSX_SDK_CACHE)
    if cached is None:
        raise SystemExit(f"extracted {tar_path} but no valid MacOSX*.sdk under {MACOSX_SDK_CACHE}")
    print(f"==> macOS SDK ready at {cached}")
    return cached


def build_web() -> None:
    print("==> Building web apps...")
    run(["npm", "ci", "--no-fund"], cwd=WEB)
    run(["npm", "run", "build:webui-esp32"], cwd=WEB)
    run(["npm", "run", "build:webui-std"], cwd=WEB)
    run(["npm", "run", "build:flasher"], cwd=WEB)
    shutil.copy2(WEB / "apps" / "flasher" / "dist" / "index.html", RELEASE / "flasher.html")
    run(["npm", "run", "build:motor-control"], cwd=WEB)
    shutil.copy2(
        WEB / "apps" / "motor-control" / "dist" / "index.html",
        RELEASE / "motor-control.html",
    )


def build_wasm_html() -> None:
    print("==> Building ossm-wasm crate, then harness HTML...")
    rustup_add("wasm32-unknown-unknown")
    ensure_wasm_bindgen()
    run(
        [
            "cargo",
            "+stable",
            "build",
            "-p",
            "ossm-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ]
    )
    WASM_PKG.mkdir(parents=True, exist_ok=True)
    run(
        [
            "wasm-bindgen",
            "--target",
            "web",
            "--out-dir",
            str(WASM_PKG),
            str(ROOT / "target" / "wasm32-unknown-unknown" / "release" / "ossm_wasm.wasm"),
        ]
    )
    run(["npm", "run", "build:html", "-w", "webui-wasm"], cwd=WEB)
    shutil.copy2(WEB / "apps" / "webui-wasm" / "dist" / "index.html", RELEASE / "ossm-wasm.html")


def build_std(*, skip_macos: bool) -> None:
    print("==> Cross-compiling ossm-std with cargo zigbuild...")
    targets = list(STD_TARGETS)
    if skip_macos:
        targets = [t for t in targets if t[1] != "aarch64-apple-darwin"]
        print("    skipping ossm-std-macos-arm64 (--skip-macos; needs MacOSX.sdk / IOKit)")
    triples = [triple for _, triple, _ in targets]
    rustup_add(*triples)
    sdkroot = None
    if any(triple == "aarch64-apple-darwin" for _, triple, _ in targets):
        sdkroot = ensure_macos_sdk()
    env = zigbuild_env(sdkroot=sdkroot)
    for dest_name, triple, bin_name in targets:
        try:
            run(
                [
                    "cargo",
                    "+stable",
                    "zigbuild",
                    "-p",
                    "ossm-std",
                    "--release",
                    "--target",
                    triple,
                ],
                env=env,
            )
        except subprocess.CalledProcessError as e:
            extra = ""
            if triple == "aarch64-apple-darwin":
                extra = (
                    " serialport needs IOKit/CoreFoundation. Set SDKROOT to a MacOSX.sdk "
                    "(or run this target on macOS). Pass --skip-macos only if you cannot provide an SDK."
                )
            raise SystemExit(
                f"zigbuild failed for {triple} ({dest_name}).{extra} "
                "Do not skip other targets."
            ) from e
        src = ROOT / "target" / triple / "release" / bin_name
        if not src.is_file():
            raise SystemExit(f"missing zigbuild output {src}")
        shutil.copy2(src, RELEASE / dest_name)
        print(f"    copied {dest_name}")


def build_firmware() -> None:
    print("==> Building firmware for ESP32-C6 ...")
    firmware_bash("cargo b-c6 --release")
    print("==> Building firmware for ESP32-S3 ...")
    firmware_bash("cargo b-s3 --release")
    print("==> Generating merged flash image for ESP32-C6...")
    run(
        [
            "espflash",
            "save-image",
            "--chip",
            "esp32c6",
            "--merge",
            str(ROOT / "target" / "riscv32imac-unknown-none-elf" / "release" / "ossm-rust"),
            str(RELEASE / "ossm-esp32c6.bin"),
        ]
    )
    print("==> Generating merged flash image for ESP32-S3...")
    run(
        [
            "espflash",
            "save-image",
            "--chip",
            "esp32s3",
            "--merge",
            str(ROOT / "target" / "xtensa-esp32s3-none-elf" / "release" / "ossm-rust"),
            str(RELEASE / "ossm-esp32s3.bin"),
        ]
    )
    print("==> Verifying ESP32-S3 flash image (single DROM segment)...")
    run([sys.executable, str(ROOT / "scripts" / "verify_s3_flash_image.py"), str(RELEASE / "ossm-esp32s3.bin")])


def main() -> None:
    load_dotenv(ROOT / ".env")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-firmware", action="store_true")
    parser.add_argument("--skip-std", action="store_true")
    parser.add_argument("--skip-web", action="store_true")
    parser.add_argument(
        "--skip-macos",
        action="store_true",
        help="Skip aarch64-apple-darwin even if MACOSX_SDK_URL / SDKROOT is available",
    )
    args = parser.parse_args()

    os.chdir(ROOT)
    prepare_release_dir()

    if not args.skip_web:
        build_web()
        build_wasm_html()
    if not args.skip_std:
        if args.skip_web:
            run(["npm", "ci", "--no-fund"], cwd=WEB)
            run(["npm", "run", "build:webui-std"], cwd=WEB)
        build_std(skip_macos=args.skip_macos)
    if not args.skip_firmware:
        build_firmware()

    print("")
    print(f"Release artifacts in {RELEASE}/:")
    for p in sorted(RELEASE.iterdir()):
        size = p.stat().st_size
        print(f"  {p.name:28} {size:>12}")


if __name__ == "__main__":
    main()
