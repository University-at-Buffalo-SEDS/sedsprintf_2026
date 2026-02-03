#!/usr/bin/env python3
import importlib.util
import os
import subprocess
import sys
from pathlib import Path


def run(cmd: list[str]) -> None:
    print("Running:", " ".join(cmd))
    subprocess.run(cmd, check=True)


def import_and_run_update():
    """Import and run subtree_update_no_stash.py directly."""
    script_path = Path(__file__).parent / "subtree_update_no_stash.py"
    if not script_path.exists():
        sys.exit(f"Error: {script_path} not found")

    spec = importlib.util.spec_from_file_location("update_subtree_no_stash", script_path)
    if spec is None or spec.loader is None:
        sys.exit("Error: could not load subtree_update_no_stash.py")

    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    if hasattr(module, "main"):
        print("→ Running subtree_update_no_stash.main()")
        module.main()
    else:
        sys.exit("Error: subtree_update_no_stash.py has no main() function")


def main():
    # ensure we are in the correct directory
    repo_root = Path(__file__).parent.resolve()
    os.chdir(f"{repo_root}/..")
    # 1. Stash any uncommitted changes
    print("→ Stashing changes...")
    run(["git", "stash"])

    # 2. Run the other script
    import_and_run_update()

    # 3. Pop stash after update
    print("→ Restoring stash...")
    try:
        run(["git", "stash", "pop"])
    except subprocess.CalledProcessError as e:
        if e.returncode == 1:
            pass
        else:
            raise


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n\nexiting...")
        exit(0)
