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


def is_git_repo(path: Path) -> bool:
    """
    Detect whether `path` is a git repository (normal repo, submodule, or worktree).
    """
    if (path / ".git").is_dir():
        return True
    if (path / ".git").is_file():  # submodules / worktrees
        return True

    # Fallback: ask git directly
    try:
        subprocess.run(
            ["git", "rev-parse", "--is-inside-work-tree"],
            cwd=path,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
        return True
    except subprocess.CalledProcessError:
        return False


def main() -> None:
    # Script lives in <repo_root>/sedsprintf_rs/update_submodule.py
    script_dir = Path(__file__).parent.resolve()
    repo_root = script_dir.parent
    os.chdir(repo_root)

    prefix = "sedsprintf_rs"
    submodule_path = repo_root / prefix

    # --- Safety checks -----------------------------------------------------

    # Ensure path exists
    if not submodule_path.exists():
        raise SystemExit(f"Path '{prefix}' does not exist")

    # Ensure it is a git repo (i.e., not a subtree)
    if not is_git_repo(submodule_path):
        raise SystemExit(
            f"'{prefix}' is NOT a git repository.\n"
            "This looks like a subtree or a plain directory.\n"
            "Refusing to run submodule update logic."
        )

    # Ensure it is actually registered as a submodule
    try:
        get_config(f"submodule.{prefix}.url")
    except SystemExit:
        raise SystemExit(
            f"'{prefix}' is a git repo, but NOT registered as a submodule.\n"
            "If this is a subtree, use the subtree updater instead."
        )

    # --- Normal submodule update logic ------------------------------------

    url = get_config(f"submodule.{prefix}.url")
    branch = get_config(f"submodule.{prefix}.branch")

    print(f"Updating submodule: {prefix}")
    print(f"  url:    {url}")
    print(f"  branch: {branch}")

    # Ensure the submodule is initialized
    run(["git", "submodule", "update", "--init", "--", prefix])

    # Fetch latest from origin
    run(["git", "fetch", "origin", branch], cwd=submodule_path)

    # Ensure correct branch
    existing = run(["git", "branch", "--list", branch], capture=True, cwd=submodule_path)
    if existing:
        run(["git", "checkout", branch], cwd=submodule_path)
    else:
        run(["git", "checkout", "-b", branch, f"origin/{branch}"], cwd=submodule_path)

    # Fast-forward only
    run(["git", "merge", "--ff-only", f"origin/{branch}"], cwd=submodule_path)

    # Stage submodule pointer
    run(["git", "add", prefix])

    # Commit only if changed
    diff_proc = subprocess.run(
        ["git", "diff", "--cached", "--quiet", "--", prefix]
    )
    if diff_proc.returncode == 0:
        print("Submodule is already at latest commit; nothing to commit.")
        return

    commit_msg = f"chore(submodule): Update submodule {prefix} to latest {branch}"
    run(["git", "commit", "-m", commit_msg])
    print("Done:", commit_msg)


if __name__ == "__main__":
    main()
