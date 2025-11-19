#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from typing import List
import os
from pathlib import Path


def run(cmd: List[str], *, capture: bool = False) -> str | None:
    """Run a command. Optionally capture stdout."""
    print("Running:", " ".join(cmd))
    if capture:
        result = subprocess.run(cmd, check=True, text=True, capture_output=True)
        return result.stdout.strip()
    else:
        subprocess.run(cmd, check=True)
        return None


def get_config(key: str) -> str:
    """Get a git config value or die with a friendly message."""
    try:
        value = run(["git", "config", "--get", key], capture=True)
    except subprocess.CalledProcessError:
        raise SystemExit(f"Missing git config key: {key}")
    if not value:
        raise SystemExit(f"Empty git config key: {key}")
    return value


def main() -> None:
    # ensure we are in the correct directory
    repo_root = Path(__file__).parent.resolve()
    os.chdir(f"{repo_root}/..")

    prefix = "sedsprintf_rs"

    remote = get_config(f"subtree.{prefix}.remote")
    branch = get_config(f"subtree.{prefix}.branch")

    print(f"Using subtree remote: {remote}")
    print(f"Using subtree branch: {branch}")

    run([
        "git",
        "subtree",
        "pull",
        "--prefix", prefix,
        remote,
        branch,
        "-m",
        f"Merge {prefix} upstream {branch}",
    ])


if __name__ == "__main__":
    main()
