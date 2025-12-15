#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import shutil
import subprocess
from pathlib import Path
import sys

# The line pattern in .gitignore you want to temporarily comment out.
# This is treated as a literal and turned into a whole-line regex.
PYI_IGNORE_LINE = "python-files/sedsprintf_rs_2026/sedsprintf_rs_2026.pyi"
PYI_IGNORE_REGEX = re.compile(rf"^{re.escape(PYI_IGNORE_LINE)}$")


def print_help(error: str | None = None) -> None:
    """
    Print usage/help text. If `error` is provided, print it first to stderr
    and exit with status 1. Otherwise, print to stdout and exit with status 0.
    """
    out = sys.stderr if error else sys.stdout

    if error:
        print(f"error: {error}\n", file=out)

    print(
        """Usage:
  build.py [OPTIONS]

Options (can be combined where it makes sense):
  release             Build in release mode.
  test                Run `cargo test` (and in test mode also validate embedded+python builds).
  embedded            Build for the embedded target (enables `embedded` feature).
  python              Build with Python bindings (enables `python` feature).
  maturin-build       Run `maturin build` with the .pyi .gitignore hack.
  maturin-develop     Run `maturin develop` with the .pyi .gitignore hack.
  maturin-install     Build wheel and install it with `uv pip install`.
  target=<triple>     Set Rust compilation target (e.g. target=thumbv7em-none-eabihf).
  device_id=<id>      Set DEVICE_IDENTIFIER env var for the build.

Special:
  -h, --help, help    Show this help message and exit.

Examples:
  build.py release
  build.py embedded release target=thumbv7em-none-eabihf
  build.py python
  build.py test
  build.py test release
  build.py maturin-build
  build.py maturin-install
""",
        file=out,
        end="",
    )

    sys.exit(1 if error else 0)


def ensure_rust_target_installed(target: str) -> None:
    """Ensure the given Rust target is installed via rustup."""
    if not target:
        return

    try:
        result = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        print(
            "warning: `rustup` not found; cannot ensure Rust target is installed. "
            "Build may fail if the target is missing.",
            file=sys.stderr,
        )
        return

    installed = {line.strip() for line in result.stdout.splitlines() if line.strip()}
    if target in installed:
        print(f"info: Rust target `{target}` already installed.")
        return

    print(f"info: Rust target `{target}` not installed; running `rustup target add {target}`")
    subprocess.run(["rustup", "target", "add", target], check=True)
    print(f"info: Successfully installed Rust target `{target}`.")


def _comment_out_pyi_ignore(gitignore: Path) -> None:
    """
    In-place edit of .gitignore using pure Python + regex:
    comment out any line whose stripped content matches PYI_IGNORE_REGEX,
    as long as it's not already commented out.
    """
    if not gitignore.exists():
        return

    text = gitignore.read_text(encoding="utf-8").splitlines(keepends=True)
    new_lines = []
    changed = False
    matched_lines = []

    for line in text:
        stripped = line.strip()

        if stripped.startswith("#"):
            new_lines.append(line)
            continue

        if PYI_IGNORE_REGEX.fullmatch(stripped):
            commented = "# " + line.lstrip()
            new_lines.append(commented)
            matched_lines.append(stripped)
            changed = True
        else:
            new_lines.append(line)

    if changed:
        print(f"info: Commented out in .gitignore: {matched_lines}")
        gitignore.write_text("".join(new_lines), encoding="utf-8")


def run_with_pyi_unignored(cmd: list[str], env: dict | None = None) -> None:
    """
    Temporarily comment out the PYI_IGNORE_LINE in .gitignore using pure Python,
    run the given command, and then restore .gitignore.
    """
    gitignore = Path(".gitignore")
    backup = None

    try:
        if gitignore.exists():
            backup = gitignore.with_name(".gitignore.maturin-backup")
            shutil.copy2(gitignore, backup)

            print(f"info: Temporarily commenting out pattern '{PYI_IGNORE_LINE}' in .gitignore")
            _comment_out_pyi_ignore(gitignore)

        print("Running:", " ".join(cmd))
        subprocess.run(cmd, check=True, env=env)
    finally:
        if backup and backup.exists():
            print("info: Restoring original .gitignore")
            shutil.move(backup, gitignore)


def install_wheel_file(build_mode: list[str], env: dict | None = None) -> None:
    cmd_build = ["maturin", "build", *build_mode]
    run_with_pyi_unignored(cmd_build, env=env)

    wheels_dir = Path("target") / "wheels"
    wheels = sorted(wheels_dir.glob("sedsprintf_rs_2026-*.whl"))
    if not wheels:
        raise SystemExit(f"No wheels found in {wheels_dir}")
    wheel = wheels[-1]

    cmd_install = ["uv", "pip", "install", str(wheel)]
    print("Running:", " ".join(cmd_install))
    subprocess.run(cmd_install, check=True, env=env)


def main(argv: list[str]) -> None:
    repo_root = Path(__file__).parent.resolve()
    os.chdir(repo_root)

    build_mode: list[str] = []
    tests = False
    build_embedded = False
    build_python = False
    build_wheel = False
    develop_wheel = False
    release_build = False
    install_wheel = False
    target = ""
    device_id = ""

    for arg in argv:
        if arg in ("-h", "--help", "help"):
            print_help()
        elif arg == "release":
            print("Building release version.")
            release_build = True
        elif arg == "test":
            print("Running tests.")
            tests = True
        elif arg == "embedded":
            print("Building for Embedded target.")
            build_embedded = True
        elif arg == "python":
            print("Building Python bindings.")
            build_python = True
        elif arg == "maturin-build":
            print("Building Python wheel.")
            build_wheel = True
        elif arg == "maturin-develop":
            print("Building and installing Python wheel in development mode.")
            develop_wheel = True
        elif arg == "maturin-install":
            print("Installing Python wheel.")
            install_wheel = True
        elif arg.startswith("target="):
            target = arg.split("=", 1)[1]
            print(f"Target set to: {target}")
        elif arg.startswith("device_id="):
            device_id = arg.split("=", 1)[1]
            print(f"Device identifier set to: {device_id}")
        else:
            print_help(f"Unknown option: {arg}")

    env = os.environ.copy()
    if device_id:
        env["DEVICE_IDENTIFIER"] = device_id

    # Release mode flags (host builds)
    if release_build:
        build_mode = ["--release"]

    # Helper to run commands
    def run_cmd(cmd: list[str]) -> None:
        print("Running:", " ".join(cmd))
        subprocess.run(cmd, check=True, env=env)

    # ---- TEST MODE: also validate embedded + python builds ----
    if tests:
        print ("--------------------------------------------")
        # 1) host tests
        print ("Running Tests tests")
        print ("--------------------------------------------")
        run_cmd(["cargo", "test"])
        print ("--------------------------------------------")
        # 2) host build (same feature) to ensure build passes even if tests were filtered
        print("Ensuring python build passes...")
        print("--------------------------------------------")
        run_cmd(["cargo", "build", "--features", "python"])
        print ("--------------------------------------------")

        # 3) embedded build (always validate)
        print("Ensuring embedded build passes...")
        print ("--------------------------------------------")
        embedded_target = target or "thumbv7em-none-eabihf"
        ensure_rust_target_installed(embedded_target)

        embedded_mode: list[str] = []
        if release_build:
            # embedded uses its own profile when in "release" mode (your existing behavior)
            embedded_mode = ["--profile", "release-embedded"]

        run_cmd([
            "cargo", "build",
            *embedded_mode,
            "--no-default-features",
            "--target", embedded_target,
            "--features", "embedded",
        ])
        print ("--------------------------------------------")

        return

    # ---- Non-test modes: preserve your existing behavior ----

    # Ensure target installed only when needed
    ensure_rust_target_installed(
        target if target else ("thumbv7em-none-eabihf" if build_embedded else "")
    )

    build_args: list[str] = []
    if build_embedded:
        if not target:
            print("info: no target specified using thumbv7em-none-eabihf")
            target = "thumbv7em-none-eabihf"
        build_args = [
            "--no-default-features",
            "--target",
            target,
            "--features",
            "embedded",
        ]
        if release_build:
            build_mode = ["--profile", "release-embedded"]
    elif build_python:
        build_args = [
            "--features",
            "python",
        ]
    else:
        if target:
            build_args = [
                "--target",
                target,
            ]

    if build_wheel:
        run_with_pyi_unignored(["maturin", "build", *build_mode], env=env)
    elif develop_wheel:
        run_with_pyi_unignored(["maturin", "develop", *build_mode], env=env)
    elif install_wheel:
        install_wheel_file(build_mode, env=env)
    else:
        run_cmd(["cargo", "build", *build_mode, *build_args])


if __name__ == "__main__":
    main(sys.argv[1:])
