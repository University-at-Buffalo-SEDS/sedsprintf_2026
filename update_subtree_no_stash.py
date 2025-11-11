#!/usr/bin/env python3
import subprocess


def run(cmd: list[str]) -> str:
    """Run a command and return its stdout (stripped)."""
    print("Running:", " ".join(cmd))
    result = subprocess.run(cmd, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def main() -> None:
    # Get the current branch name
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    print(f"Current branch: {branch}")

    # Pull that branch from the upstream
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
