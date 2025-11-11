#!/usr/bin/env python3
import subprocess
from typing import List, Optional


def run(cmd: List[str], *, capture: bool = False) -> Optional[str]:
    """Run a command. If capture=True, return its stdout (stripped)."""
    print("Running:", " ".join(cmd))
    if capture:
        result = subprocess.run(cmd, check=True, text=True, capture_output=True)
        return result.stdout.strip()
    else:
        subprocess.run(cmd, check=True)  # inherits stdout/stderr → keeps colors
        return None


def main() -> None:
    # Find the subtree remote's currently active branch
    branches = run(["git", "branch", "-r"], capture=True).splitlines()
    subtree_branches = [b.strip() for b in branches if "sedsprintf-upstream/" in b]

    if not subtree_branches:
        raise SystemExit("No branches found for remote 'sedsprintf-upstream'")

    head_line = next((b for b in subtree_branches if "HEAD ->" in b), None)
    if head_line:
        branch = head_line.split("-> sedsprintf-upstream/")[-1].strip()
    else:
        branch = subtree_branches[0].split("sedsprintf-upstream/")[-1].strip()

    print(f"Detected subtree branch: {branch}")

    # Let git subtree pull talk directly to the terminal → colors preserved
    run([
        "git",
        "subtree",
        "pull",
        "--prefix=sedsprintf_rs",
        "sedsprintf-upstream",
        branch,
        "-m",
        f"Merge sedsprintf_rs upstream {branch}",
    ])


if __name__ == "__main__":
    main()
