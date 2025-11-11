#!/usr/bin/env python3
import subprocess


def run(cmd: list[str]) -> str:
    """Run a command and return its stdout (stripped)."""
    print("Running:", " ".join(cmd))
    result = subprocess.run(cmd, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def main() -> None:
    # Find the subtree remote's currently active branch
    branches = run(["git", "branch", "-r"]).splitlines()
    subtree_branches = [b.strip() for b in branches if "sedsprintf-upstream/" in b]

    if not subtree_branches:
        raise SystemExit("No branches found for remote 'sedsprintf-upstream'")

    # Try to detect the currently checked-out one
    # e.g. if output looks like: 'sedsprintf-upstream/HEAD -> sedsprintf-upstream/DMA'
    head_line = next((b for b in subtree_branches if "HEAD ->" in b), None)
    if head_line:
        branch = head_line.split("-> sedsprintf-upstream/")[-1].strip()
    else:
        # fallback: pick the first non-HEAD entry
        branch = subtree_branches[0].split("sedsprintf-upstream/")[-1].strip()

    print(f"Detected subtree branch: {branch}")

    # Perform the subtree pull
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
