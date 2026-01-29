#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

# The line pattern in .gitignore we want to temporarily comment out.
# This is treated as a literal and turned into a whole-line regex.
PYI_IGNORE_LINE = "python-files/sedsprintf_rs/sedsprintf_rs.pyi"
PYI_IGNORE_REGEX = re.compile(rf"^{re.escape(PYI_IGNORE_LINE)}$")


def print_help(error: str | None = None) -> None:
    out = sys.stderr if error else sys.stdout
    if error:
        print(f"error: {error}\n", file=out)

    print(
        """Usage:
  build.py [OPTIONS]

Options (can be combined where it makes sense):
  release                 Build in release mode.
  test                    Run `cargo test` (and in test mode also validate embedded+python builds).
  embedded                Build for the embedded target (enables `embedded` feature).
  python                  Build with Python bindings (enables `python` feature).
  maturin-build           Run `maturin build` with the .pyi .gitignore hack.
  maturin-develop         Run `maturin develop` with the .pyi .gitignore hack.
  maturin-install         Build wheel and install it with `uv pip install`.
  target=<triple>         Set Rust compilation target (e.g. target=thumbv7em-none-eabihf).
  device_id=<id>          Set DEVICE_IDENTIFIER env var for the build.
  schema_path=<path>      Set SEDSPRINTF_RS_SCHEMA_PATH for the build.

New (compile-time env vars):
  max_stack_payload=<n>   Set MAX_STACK_PAYLOAD for define_stack_payload!(env="MAX_STACK_PAYLOAD", ...).
  env:KEY=VALUE           Set arbitrary environment variable(s) for the build (repeatable).
                          Example: env:MAX_QUEUE_SIZE=65536 env:QUEUE_GROW_STEP=2.0

Special:
  -h, --help, help        Show this help message and exit.

Examples:
  build.py release
  build.py embedded release target=thumbv7em-none-eabihf
  build.py python
  build.py test
  build.py test release
  build.py maturin-build max_stack_payload=256
  build.py maturin-install env:MAX_RECENT_RX_IDS=256 env:MAX_STACK_PAYLOAD=128
""",
        file=out,
        end="",
    )
    sys.exit(1 if error else 0)


def _fmt_secs(s: float) -> str:
    if s < 1:
        return f"{s * 1000:.0f}ms"
    if s < 60:
        return f"{s:.2f}s"
    m = int(s // 60)
    r = s - 60 * m
    return f"{m}m{r:.0f}s"


def _banner(title: str) -> None:
    print("\n" + "-" * 60)
    print(title)
    print("-" * 60)


def _success(msg: str) -> None:
    print(f"✅ {msg}")


def _fail(msg: str) -> None:
    print(f"❌ {msg}", file=sys.stderr)


def output_hint_for_cmd(
        cmd: list[str],
        *,
        repo_root: Path,
        target: str = "",
        release_build: bool = False,
        embedded_profile: bool = False,
) -> str | None:
    """
    Best-effort output location hint for common commands.
    """
    if not cmd:
        return None

    if cmd[:2] == ["cargo", "test"]:
        # cargo test builds test artifacts (and deps) under target/...
        return f"Build artifacts: {repo_root / 'target'}"

    if cmd[:2] == ["cargo", "build"]:
        # Determine target dir
        tgt = ""
        if "--target" in cmd:
            try:
                tgt = cmd[cmd.index("--target") + 1]
            except Exception:
                tgt = ""
        tgt = tgt or target

        # Determine profile
        prof = None
        if "--profile" in cmd:
            try:
                prof = cmd[cmd.index("--profile") + 1]
            except Exception:
                prof = None

        if prof:
            # Custom profile output is still under target/<target?>/<profile> (cargo profile name)
            if tgt:
                return f"Output: {repo_root / 'target' / tgt / prof}"
            return f"Output: {repo_root / 'target' / prof}"

        # Standard debug/release
        std = "release" if release_build else "debug"
        if embedded_profile:
            # Your embedded release uses --profile release-embedded
            # but if embedded_profile is True we already handled prof above.
            pass
        if tgt:
            return f"Output: {repo_root / 'target' / tgt / std}"
        return f"Output: {repo_root / 'target' / std}"

    if cmd and cmd[0] == "maturin" and len(cmd) >= 2:
        if cmd[1] == "build":
            return f"Wheels: {repo_root / 'target' / 'wheels'}"
        if cmd[1] == "develop":
            return "Installs into the active Python environment (editable/dev install)."

    if cmd[:3] == ["uv", "pip", "install"]:
        return "Installed into the active Python environment."

    return None


def run_cmd(
        cmd: list[str],
        *,
        env: dict[str, str],
        repo_root: Path,
        title: str | None = None,
        target: str = "",
        release_build: bool = False,
        embedded_profile: bool = False,
) -> None:
    """
    Run a command with nicer user feedback:
    - Section banner
    - Timing
    - PASS/FAIL
    - Output location hints
    """
    if title:
        _banner(title)
    else:
        _banner("Running: " + " ".join(cmd))

    hint = output_hint_for_cmd(
        cmd,
        repo_root=repo_root,
        target=target,
        release_build=release_build,
        embedded_profile=embedded_profile,
    )
    if hint:
        print(f"info: {hint}")

    print("Running:", " ".join(cmd))
    t0 = time.monotonic()
    try:
        subprocess.run(cmd, check=True, env=env)
    except subprocess.CalledProcessError as e:
        dt = time.monotonic() - t0
        _fail(f"FAILED ({_fmt_secs(dt)}): {' '.join(cmd)}")
        # Keep the same exit code
        raise SystemExit(e.returncode) from e
    dt = time.monotonic() - t0
    _success(f"OK ({_fmt_secs(dt)}): {' '.join(cmd)}")


def ensure_rust_target_installed(target: str) -> None:
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

    _banner(f"Installing Rust target: {target}")
    subprocess.run(["rustup", "target", "add", target], check=True)
    _success(f"Installed Rust target `{target}`.")


def _comment_out_pyi_ignore(gitignore: Path) -> None:
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


def run_with_pyi_unignored(
        cmd: list[str],
        *,
        env: dict[str, str],
        repo_root: Path,
        title: str | None = None,
) -> None:
    gitignore = Path(".gitignore")
    backup = None

    if title:
        _banner(title)
    else:
        _banner("Running: " + " ".join(cmd))

    hint = output_hint_for_cmd(cmd, repo_root=repo_root)
    if hint:
        print(f"info: {hint}")

    t0 = time.monotonic()
    try:
        if gitignore.exists():
            backup = gitignore.with_name(".gitignore.maturin-backup")
            shutil.copy2(gitignore, backup)

            print(f"info: Temporarily commenting out pattern '{PYI_IGNORE_LINE}' in .gitignore")
            _comment_out_pyi_ignore(gitignore)

        print("Running:", " ".join(cmd))
        subprocess.run(cmd, check=True, env=env)

    except subprocess.CalledProcessError as e:
        dt = time.monotonic() - t0
        _fail(f"FAILED ({_fmt_secs(dt)}): {' '.join(cmd)}")
        raise SystemExit(e.returncode) from e

    finally:
        if backup and backup.exists():
            print("info: Restoring original .gitignore")
            shutil.move(backup, gitignore)

    dt = time.monotonic() - t0
    _success(f"OK ({_fmt_secs(dt)}): {' '.join(cmd)}")


def install_wheel_file(build_mode: list[str], *, env: dict[str, str], repo_root: Path) -> None:
    run_with_pyi_unignored(
        ["maturin", "build", *build_mode],
        env=env,
        repo_root=repo_root,
        title="maturin build (wheel)",
    )

    wheels_dir = repo_root / "target" / "wheels"
    wheels = sorted(wheels_dir.glob("sedsprintf_rs-*.whl"))
    if not wheels:
        raise SystemExit(f"No wheels found in {wheels_dir}")
    wheel = wheels[-1]

    _banner("Installing wheel with uv")
    print(f"info: Selected wheel: {wheel}")
    run_cmd(
        ["uv", "pip", "install", str(wheel)],
        env=env,
        repo_root=repo_root,
        title="uv pip install (wheel)",
    )
    _success("Wheel installed.")
    print(f"info: Wheel file remains at: {wheel}")


def _apply_env_overrides(env: dict[str, str], overrides: dict[str, str]) -> None:
    for k, v in overrides.items():
        env[k] = v
        print(f"info: env override: {k}={v}")


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
    schema_path = ""

    env_overrides: dict[str, str] = {}

    for arg in argv:
        if arg in ("-h", "--help", "help"):
            print_help()

        elif arg == "release":
            print("Building release version.")
            release_build = True

        elif arg == "test":
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

        elif arg.startswith("schema_path="):
            schema_path = arg.split("=", 1)[1]
            if not schema_path:
                print_help("schema_path requires a value")
            env_overrides["SEDSPRINTF_RS_SCHEMA_PATH"] = schema_path

        elif arg.startswith("max_stack_payload="):
            v = arg.split("=", 1)[1].strip()
            if not v:
                print_help("max_stack_payload requires a value")
            env_overrides["MAX_STACK_PAYLOAD"] = v

        elif arg.startswith("env:"):
            rest = arg[4:]
            if "=" not in rest:
                print_help("env:KEY=VALUE requires '='")
            k, v = rest.split("=", 1)
            k = k.strip()
            v = v.strip()
            if not k:
                print_help("env:KEY=VALUE requires a non-empty KEY")
            env_overrides[k] = v

        else:
            print_help(f"Unknown option: {arg}")

    env = os.environ.copy()
    if device_id:
        env["DEVICE_IDENTIFIER"] = device_id
    if tests:
        if "SEDSPRINTF_RS_SCHEMA_PATH" not in env_overrides and "SEDSPRINTF_RS_SCHEMA_PATH" not in env:
            env_overrides["SEDSPRINTF_RS_SCHEMA_PATH"] = str(
                (repo_root / "telemetry_config.test.json").resolve()
            )
    _apply_env_overrides(env, env_overrides)

    if release_build:
        build_mode = ["--release"]

    # ---- TEST MODE: also validate embedded + python builds ----
    if tests:
        _banner("TEST MODE")

        run_cmd(
            ["cargo", "test"],
            env=env,
            repo_root=repo_root,
            title="1/3 cargo test",
            release_build=release_build,
        )
        _success("Tests passed.")

        run_cmd(
            ["cargo", "build", "--features", "python", *build_mode],
            env=env,
            repo_root=repo_root,
            title="2/3 cargo build (python feature)",
            release_build=release_build,
        )
        _success(f"Python-feature build finished. Output is under: {repo_root / 'target'}")

        embedded_target = target or "thumbv7em-none-eabihf"
        ensure_rust_target_installed(embedded_target)

        embedded_mode: list[str] = []
        embedded_profile_name: str | None = None
        if release_build:
            embedded_mode = ["--profile", "release-embedded"]
            embedded_profile_name = "release-embedded"

        run_cmd(
            [
                "cargo",
                "build",
                *embedded_mode,
                "--no-default-features",
                "--target",
                embedded_target,
                "--features",
                "embedded",
            ],
            env=env,
            repo_root=repo_root,
            title="3/3 cargo build (embedded feature)",
            target=embedded_target,
            release_build=release_build,
            embedded_profile=bool(embedded_profile_name),
        )

        if embedded_profile_name:
            print(f"info: Embedded output: {repo_root / 'target' / embedded_target / embedded_profile_name}")
        else:
            prof = "release" if release_build else "debug"
            print(f"info: Embedded output: {repo_root / 'target' / embedded_target / prof}")

        _success("All test-mode checks passed.")
        return

    # ---- Non-test modes ----
    ensure_rust_target_installed(target if target else ("thumbv7em-none-eabihf" if build_embedded else ""))

    build_args: list[str] = []
    embedded_profile = False

    if build_embedded:
        if not target:
            print("info: no target specified using thumbv7em-none-eabihf")
            target = "thumbv7em-none-eabihf"
        build_args = ["--no-default-features", "--target", target, "--features", "embedded"]
        if release_build:
            build_mode = ["--profile", "release-embedded"]
            embedded_profile = True

        run_cmd(
            ["cargo", "build", *build_mode, *build_args],
            env=env,
            repo_root=repo_root,
            title="cargo build (embedded)",
            target=target,
            release_build=release_build,
            embedded_profile=embedded_profile,
        )
        if embedded_profile:
            print(f"info: Output: {repo_root / 'target' / target / 'release-embedded'}")
        else:
            prof = "release" if release_build else "debug"
            print(f"info: Output: {repo_root / 'target' / target / prof}")
        return

    if build_python:
        build_args = ["--features", "python"]
        run_cmd(
            ["cargo", "build", *build_mode, *build_args],
            env=env,
            repo_root=repo_root,
            title="cargo build (python feature)",
            release_build=release_build,
        )
        _success(f"Build finished. Output is under: {repo_root / 'target'}")
        return

    if build_wheel:
        run_with_pyi_unignored(
            ["maturin", "build", *build_mode],
            env=env,
            repo_root=repo_root,
            title="maturin build (wheel)",
        )
        _success(f"Wheel build finished. Wheels are in: {repo_root / 'target' / 'wheels'}")
        return

    if develop_wheel:
        run_with_pyi_unignored(
            ["maturin", "develop", *build_mode],
            env=env,
            repo_root=repo_root,
            title="maturin develop (installs into env)",
        )
        _success("Develop install finished (installed into your current Python environment).")
        return

    if install_wheel:
        install_wheel_file(build_mode, env=env, repo_root=repo_root)
        return

    # default: plain cargo build
    if target:
        build_args = ["--target", target]

    run_cmd(
        ["cargo", "build", *build_mode, *build_args],
        env=env,
        repo_root=repo_root,
        title="cargo build",
        target=target,
        release_build=release_build,
    )
    if target:
        prof = "release" if release_build else "debug"
        _success(f"Build finished. Output: {repo_root / 'target' / target / prof}")
    else:
        prof = "release" if release_build else "debug"
        _success(f"Build finished. Output: {repo_root / 'target' / prof}")


if __name__ == "__main__":
    main(sys.argv[1:])
