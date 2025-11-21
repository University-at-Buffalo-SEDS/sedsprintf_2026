#!/usr/bin/env python3
from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import List, Optional


def run(cmd: List[str], *, capture: bool = False, cwd: Optional[Path] = None) -> str | None:
    """Run a command. Optionally capture stdout."""
    print("Running:", " ".join(cmd))
    if capture:
        result = subprocess.run(
            cmd,
            check=True,
            text=True,
            capture_output=True,
            cwd=str(cwd) if cwd is not None else None,
        )
        return result.stdout.strip()
    else:
        subprocess.run(cmd, check=True, cwd=str(cwd) if cwd is not None else None)
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
    # ensure we are in the correct directory (repo root, one up from this script)
    repo_root = Path(__file__).parent.resolve()
    os.chdir(repo_root / "..")

    # Path of the submodule in the parent repo
    prefix = "sedsprintf_rs"

    # Read submodule config (branch preservation)
    url = get_config(f"submodule.{prefix}.url")
    branch = get_config(f"submodule.{prefix}.branch")

    print(f"Updating submodule: {prefix}")
    print(f"  url:    {url}")
    print(f"  branch: {branch}")

    # Make sure the submodule exists & is initialized
    run(["git", "submodule", "update", "--init", "--", prefix])

    submodule_path = Path(prefix)

    # Fetch latest from origin for that branch
    run(["git", "fetch", "origin", branch], cwd=submodule_path)

    # Ensure we are on the right branch in the submodule
    existing = run(["git", "branch", "--list", branch], capture=True, cwd=submodule_path)
    if existing:
        # Branch exists locally; just check it out
        run(["git", "checkout", branch], cwd=submodule_path)
    else:
        # Create local branch tracking origin/branch
        run(["git", "checkout", "-b", branch, f"origin/{branch}"], cwd=submodule_path)

    # Fast-forward to origin/<branch> (no merge noise)
    run(["git", "merge", "--ff-only", f"origin/{branch}"], cwd=submodule_path)

    # Stage the updated submodule pointer in the parent repo
    run(["git", "add", prefix])

    # Only commit if there is actually a change in the submodule entry
    diff_proc = subprocess.run(
        ["git", "diff", "--cached", "--quiet", "--", prefix]
    )
    if diff_proc.returncode == 0:
        print("Submodule is already at latest commit; nothing to commit.")
        return

    commit_msg = f"Update submodule {prefix} to latest {branch}"
    run(["git", "commit", "-m", commit_msg])
    print("Done:", commit_msg)


if __name__ == "__main__":
    main()
