#!/usr/bin/env python3
from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import List, Optional


def run(cmd: List[str], *, capture: bool = False) -> Optional[str]:
    """Run a command. Optionally capture stdout."""
    print("Running:", " ".join(cmd))
    if capture:
        result = subprocess.run(cmd, check=True, text=True, capture_output=True)
        return result.stdout.strip()
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


def get_config_default(key: str, default: str) -> str:
    try:
        return get_config(key)
    except SystemExit:
        return default


def is_git_repo(path: Path) -> bool:
    return (path / ".git").exists()


def main() -> None:
    # Script should live in: <super_repo_root>/sedsprintf_rs/update_subtree.py
    subtree_prefix = Path(__file__).parent.resolve()
    super_repo_root = subtree_prefix.parent
    subtree_name = subtree_prefix.name  # "sedsprintf_rs"

    if is_git_repo(subtree_prefix):
        raise SystemExit(
            f"'{subtree_name}' looks like its own git repo (has .git). "
            "That is a submodule/standalone clone, not a subtree. "
            "Remove the nested .git directory and recommit, or fix your layout."
        )

    os.chdir(super_repo_root)

    branch = get_config_default(f"subtree.{subtree_name}.branch", "main")
    squash_raw = get_config_default(f"subtree.{subtree_name}.squash", "false").strip().lower()
    squash = squash_raw in {"1", "true", "yes", "on"}

    # Commit message template (configurable)
    msg_default = f"chore(subtree): update {subtree_name} from upstream/{branch}"
    if squash:
        msg_default += " (squashed)"

    commit_msg = get_config_default(
        f"subtree.{subtree_name}.message",
        msg_default,
    )

    print(f"Updating subtree '{subtree_name}' (prefix '{subtree_name}') branch '{branch}'")
    print(f"Squash: {squash}")
    print(f"Commit message: {commit_msg}")

    # Stable local remote name
    remote_name = "sedsprintf-upstream"

    # Fetch latest
    run(["git", "fetch", "--prune", remote_name])

    # Perform the subtree pull with commit message
    cmd = [
        "git",
        "subtree",
        "pull",
        "--prefix",
        subtree_name,
        remote_name,
        branch,
        "-m",
        commit_msg,
    ]
    if squash:
        cmd.append("--squash")

    try:
        run(cmd)
    except subprocess.CalledProcessError:
        print("Subtree pull did not apply changes (already up to date or no matching commits).")
        return

    print("Subtree updated successfully.")


if __name__ == "__main__":
    main()
