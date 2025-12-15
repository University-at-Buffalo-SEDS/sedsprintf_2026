#!/usr/bin/env python3
import importlib.util
import os
import subprocess
from pathlib import Path

import sys


def run(cmd: list[str]) -> None:
    print("Running:", " ".join(cmd))
    subprocess.run(cmd, check=True)


def import_and_run_update():
    """Import and run submodule_update.py directly."""
    script_path = Path(__file__).parent / "submodule_update_no_stash.py"
    if not script_path.exists():
        sys.exit(f"Error: {script_path} not found")

    spec = importlib.util.spec_from_file_location("submodule_update", script_path)
    if spec is None or spec.loader is None:
        sys.exit("Error: could not load submodule_update.py")

    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    if hasattr(module, "main"):
        print("→ Running submodule_update.main()")
        module.main()
    else:
        sys.exit("Error: submodule_update.py has no main() function")


def main():
    # ensure we are in the correct directory
    repo_root = Path(__file__).parent.resolve()
    os.chdir(repo_root / "..")

    # 1. Stash uncommitted changes
    print("→ Stashing changes...")
    run(["git", "stash"])

    # 2. Run updater
    import_and_run_update()

    # 3. Pop stash afterwards (same behavior as subtree scripts)
    print("→ Restoring stash...")
    try:
        run(["git", "stash", "pop"])
    except subprocess.CalledProcessError as e:
        if e.returncode == 1:
            # "No stash to apply" — safe to ignore
            pass
        else:
            raise


if __name__ == "__main__":
    main()
