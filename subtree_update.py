#!/usr/bin/env python3
import importlib.util
import os
import subprocess
import sys
from pathlib import Path


def _fail(msg: str) -> None:
    print(f"Error: {msg}", file=sys.stderr)


def _cmd_text(cmd: list[str]) -> str:
    return " ".join(cmd)


def _hint_for_git_cmd(cmd: list[str], returncode: int) -> str | None:
    if not cmd or cmd[0] != "git":
        return None
    if cmd[:2] == ["git", "stash"] and "pop" in cmd:
        return "Run `git status` to inspect conflicts, resolve them, then continue."
    if cmd[:2] == ["git", "stash"]:
        return "Ensure you are inside a git repository and have permission to modify it."
    return f"Try running `{_cmd_text(cmd)}` manually for detailed git diagnostics."


def run(cmd: list[str]) -> None:
    print("Running:", _cmd_text(cmd))
    try:
        subprocess.run(cmd, check=True)
    except FileNotFoundError as e:
        raise SystemExit(
            f"Required command '{e.filename}' was not found while running: {_cmd_text(cmd)}. "
            "Install git and ensure it is on your PATH."
        ) from e
    except subprocess.CalledProcessError as e:
        hint = _hint_for_git_cmd(cmd, e.returncode)
        raise SystemExit(
            f"Command failed with exit code {e.returncode}: {_cmd_text(cmd)}"
            + (f". Hint: {hint}" if hint else "")
        ) from e


def import_and_run_update():
    """Import and run subtree_update_no_stash.py directly."""
    script_path = Path(__file__).parent / "subtree_update_no_stash.py"
    if not script_path.exists():
        sys.exit(
            f"Error: {script_path} not found. "
            "Ensure this wrapper is run from the repository that contains subtree_update_no_stash.py."
        )

    spec = importlib.util.spec_from_file_location("update_subtree_no_stash", script_path)
    if spec is None or spec.loader is None:
        sys.exit(
            "Error: could not load subtree_update_no_stash.py. "
            "Check file permissions and that the script is valid Python."
        )

    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    if hasattr(module, "main"):
        print("→ Running subtree_update_no_stash.main()")
        module.main()
    else:
        sys.exit(
            "Error: subtree_update_no_stash.py has no main() function. "
            "Restore that entrypoint or run the no-stash script directly after fixing it."
        )


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
    stash_pop = subprocess.run(["git", "stash", "pop"], check=False)
    if stash_pop.returncode == 0:
        return
    if stash_pop.returncode == 1:
        # "No stash entries found." is safe to ignore.
        return
    raise SystemExit(
        "Failed to restore stashed changes with `git stash pop`. "
        "Run `git stash list` and `git stash pop` manually to resolve."
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n\nexiting...")
        exit(0)
    except Exception as e:
        _fail(f"Unexpected error: {e}")
        raise SystemExit(1) from e
