#!/usr/bin/env python3
from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import List


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
    # Script is located inside the submodule working tree:
    #   <super_repo_root>/sedsprintf_rs/update_subtree.py (renamed use)
    submodule_root = Path(__file__).parent.resolve()
    super_repo_root = submodule_root.parent
    submodule_name = submodule_root.name  # "sedsprintf_rs"

    # Determine which branch the submodule should track.
    # Prefer submodule.<name>.branch from the superproject, else default to "main".
    os.chdir(super_repo_root)
    try:
        branch = get_config(f"submodule.{submodule_name}.branch")
    except SystemExit:
        branch = "main"
        print(f"submodule.{submodule_name}.branch not set; defaulting to '{branch}'")

    print(f"Updating submodule '{submodule_name}' to latest '{branch}'")

    # Step 1: Update the submodule itself.
    os.chdir(submodule_root)

    # Ensure we have the latest from the remote.
    run(["git", "fetch", "origin"])

    # Check out the desired branch (creating local branch if necessary),
    # then fast-forward to origin/<branch>.
    try:
        # Try to check out existing local branch
        run(["git", "checkout", branch])
    except subprocess.CalledProcessError:
        # If it doesn't exist locally, create it to track origin/<branch>
        print(f"Local branch '{branch}' not found; creating to track origin/{branch}")
        run(["git", "checkout", "-b", branch, f"origin/{branch}"])

    # Fast-forward to the latest remote commit
    run(["git", "pull", "--ff-only", "origin", branch])

    # Step 2: Record the new submodule commit in the superproject.
    os.chdir(super_repo_root)

    try:
        run(
            [
                "git",
                "commit",
                "-m",
                f"Update {submodule_name} submodule to latest {branch}",
            ]
        )
    except subprocess.CalledProcessError:
        print("No changes to commit; submodule already up to date.")


if __name__ == "__main__":
    main()
