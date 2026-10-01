#!/usr/bin/env python3
# Release driver, mirroring blaz's scripts/release.py. Builds everything on
# this machine (inside the nix dev shell), updates the flake's prebuilt
# URL + hash, commits, tags, pushes and publishes the GitHub release.
# Bouedig's server artifact is a tarball (binaries + web bundle) instead of
# a bare binary because the backend loads its web client from
# BOUEDIG_STATIC_DIR at runtime.

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

APP = "bouedig"
TARGET = "x86_64-linux"
REPO = "MathieuMoalic/bouedig"

ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / "frontend-web"
ANDROID = ROOT / "frontend-android"
FLAKE = ROOT / "flake.nix"
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
RELEASE_DIR = ROOT / "release" / "artifacts"
WEB_BUNDLE = ROOT / "target" / "dx" / "frontend-web" / "release" / "web" / "public"


def run(*cmd: str, cwd: Path | None = None) -> None:
    print("+", " ".join(cmd))
    subprocess.run(cmd, cwd=cwd or ROOT, check=True)


def output(*cmd: str, cwd: Path | None = None) -> str:
    return subprocess.check_output(cmd, cwd=cwd or ROOT, text=True).strip()


def ensure_clean_tree() -> None:
    status = output("git", "status", "--short")
    if status:
        print("Error: working tree is dirty. Commit or stash changes before releasing.")
        print(status)
        sys.exit(1)


def current_version() -> str:
    text = CARGO_TOML.read_text()
    match = re.search(r'(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"$', text)
    if not match:
        raise RuntimeError("Could not find version in Cargo.toml [workspace.package]")
    return match.group(1)


def bump_version(version: str, bump_type: str) -> str:
    major, minor, patch = map(int, version.split("."))

    match bump_type:
        case "major":
            return f"{major + 1}.0.0"
        case "minor":
            return f"{major}.{minor + 1}.0"
        case "patch":
            return f"{major}.{minor}.{patch + 1}"
        case _:
            raise RuntimeError("TYPE must be major, minor, or patch")


def replace_once(text: str, pattern: str, replacement: str, label: str) -> str:
    new_text, count = re.subn(pattern, replacement, text, count=1)
    if count != 1:
        raise RuntimeError(f"Failed to update {label}")
    return new_text


def update_version_files(old: str, new: str) -> None:
    print(f"Bumping version: {old} -> {new}")

    cargo = CARGO_TOML.read_text()
    cargo = replace_once(
        cargo,
        rf'(?m)^version = "{re.escape(old)}"$',
        f'version = "{new}"',
        "Cargo.toml [workspace.package] version",
    )
    CARGO_TOML.write_text(cargo)

    flake = FLAKE.read_text()
    flake = replace_once(
        flake,
        rf'(?m)^(\s*version = "){re.escape(old)}(";)$',
        rf"\g<1>{new}\2",
        "flake.nix version",
    )
    FLAKE.write_text(flake)


def update_flake_prebuilt(version: str, nix_hash: str) -> None:
    tag = f"v{version}"
    tarball_name = f"{APP}-{tag}-{TARGET}.tar.gz"
    url = f"https://github.com/{REPO}/releases/download/{tag}/{tarball_name}"

    flake = FLAKE.read_text()
    flake = replace_once(
        flake,
        r'(?m)^(\s*prebuiltHash = ")[^"]+(";)$',
        rf"\g<1>{nix_hash}\2",
        "flake.nix prebuiltHash",
    )
    flake = replace_once(
        flake,
        r'(?m)^(\s*version = ")[0-9]+\.[0-9]+\.[0-9]+(";)$',
        rf"\g<1>{version}\2",
        "flake.nix version",
    )
    FLAKE.write_text(flake)


def cargo_check() -> None:
    run("cargo", "check", "--quiet", "-p", "shared", "-p", "backend")


def build_web() -> Path:
    run("dx", "build", "--platform", "web", "--release", cwd=WEB)
    if not (WEB_BUNDLE / "index.html").is_file():
        raise RuntimeError(f"no web bundle at {WEB_BUNDLE}")
    return WEB_BUNDLE


def build_tarball(version: str) -> Path:
    tag = f"v{version}"
    artifact = RELEASE_DIR / f"{APP}-{tag}-{TARGET}.tar.gz"

    run("cargo", "build", "--release", "--locked", "-p", "backend")
    target = ROOT / "target" / "release"
    bins = ["backend", "migrate", "import_check"]
    for bin_name in bins:
        if not (target / bin_name).is_file():
            raise RuntimeError(f"missing binary: {target / bin_name}")

    staging = RELEASE_DIR / "staging"
    if staging.exists():
        shutil.rmtree(staging)
    bin_dir = staging / "bin"
    web_dir = staging / "share" / "bouedig" / "web"
    bin_dir.mkdir(parents=True)
    web_dir.mkdir(parents=True)

    for bin_name in bins:
        dest = bin_dir / ("bouedig" if bin_name == "backend" else bin_name)
        shutil.copy2(target / bin_name, dest)
        dest.chmod(0o755)
        try:
            run("strip", str(dest))
        except (subprocess.CalledProcessError, FileNotFoundError):
            print("Warning: strip failed; keeping unstripped binary")

    shutil.copytree(WEB_BUNDLE, web_dir, dirs_exist_ok=True)

    with tarfile.open(artifact, "w:gz") as tar:
        for rel in ["bin", "share", "share/bouedig", "share/bouedig/web"]:
            tar.add(staging / rel, arcname=rel, recursive=False)
        for item in sorted((staging / "bin").iterdir()):
            tar.add(item, arcname=f"bin/{item.name}")
        for item in sorted(web_dir.rglob("*")):
            tar.add(item, arcname=f"share/bouedig/web/{item.relative_to(web_dir)}",
                    recursive=False)

    shutil.rmtree(staging)
    artifact.chmod(0o644)
    return artifact


def build_apk(version: str) -> Path:
    tag = f"v{version}"
    artifact = RELEASE_DIR / f"{APP}-{tag}.apk"

    run("dx", "build", "--platform", "android", "--release", cwd=ANDROID)

    dx_out = ROOT / "target" / "dx" / "frontend-android"
    candidates = [
        p for p in dx_out.rglob("*.apk")
        if "release" in p.parts and "-unsigned" not in p.name
    ]
    if not candidates:
        candidates = [p for p in dx_out.rglob("*.apk")]
    if not candidates:
        raise RuntimeError(f"no .apk found under {dx_out}")
    apk = sorted(candidates, key=lambda p: (0 if "universal" in p.parts else 1, str(p)))[0]

    shutil.copy2(apk, artifact)
    return artifact


def nix_hash_file(path: Path) -> str:
    return output("nix", "hash", "file", "--type", "sha256", str(path))


def commit_and_tag(version: str) -> None:
    tag = f"v{version}"
    run("git", "add", str(CARGO_TOML), str(CARGO_LOCK), str(FLAKE))
    run("git", "--no-pager", "diff", "--cached", "--stat")
    run("git", "commit", "-m", f"Release {tag}")
    run("git", "tag", "-a", tag, "-m", f"Release {tag}")


def push_release_command(tag: str) -> None:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise RuntimeError("TAG must look like v1.2.3")

    tarball = RELEASE_DIR / f"{APP}-{tag}-{TARGET}.tar.gz"
    apk = RELEASE_DIR / f"{APP}-{tag}.apk"

    if not tarball.exists():
        raise RuntimeError(f"Missing server tarball: {tarball}")
    if not apk.exists():
        raise RuntimeError(f"Missing APK: {apk}")

    run(
        "gh",
        "release",
        "create",
        tag,
        "--generate-notes",
        "--",
        str(tarball),
        str(apk),
    )


def bump_command(bump_type: str) -> None:
    old = current_version()
    new = bump_version(old, bump_type)
    update_version_files(old, new)
    cargo_check()
    print(f"Version files updated to {new}")


def release_command(bump_type: str) -> None:
    ensure_clean_tree()

    old = current_version()
    new = bump_version(old, bump_type)
    tag = f"v{new}"
    start_head = output("git", "rev-parse", "HEAD")
    pushed = False

    try:
        RELEASE_DIR.mkdir(parents=True, exist_ok=True)
        for item in RELEASE_DIR.iterdir():
            if item.is_dir():
                shutil.rmtree(item)
            else:
                item.unlink()

        update_version_files(old, new)
        cargo_check()
        build_web()
        tarball = build_tarball(new)
        nix_hash = nix_hash_file(tarball)

        print(f"Server tarball: {tarball}")
        print(f"Nix hash: {nix_hash}")

        update_flake_prebuilt(new, nix_hash)
        commit_and_tag(new)
        apk = build_apk(new)

        print("\nRelease artifacts:")
        print(f"  {tarball}")
        print(f"  {apk}")

        pushed = True
        run("git", "push", "origin", "HEAD")
        run("git", "push", "origin", tag)
        push_release_command(tag)
        print(f"\nReleased {tag}")
    except Exception:
        if not pushed:
            run("git", "reset", "--hard", start_head)
            if output("git", "tag", "-l", tag) == tag:
                run("git", "tag", "-d", tag)
            if RELEASE_DIR.exists():
                shutil.rmtree(RELEASE_DIR)
        raise


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    bump_parser = subparsers.add_parser("bump")
    bump_parser.add_argument("type", choices=["major", "minor", "patch"])

    release_parser = subparsers.add_parser("release")
    release_parser.add_argument("type", choices=["major", "minor", "patch"])

    push_parser = subparsers.add_parser("push-release")
    push_parser.add_argument("tag")

    args = parser.parse_args()

    try:
        match args.command:
            case "bump":
                bump_command(args.type)
            case "release":
                release_command(args.type)
            case "push-release":
                push_release_command(args.tag)
            case _:
                raise RuntimeError(f"Unknown command: {args.command}")
    except Exception as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
