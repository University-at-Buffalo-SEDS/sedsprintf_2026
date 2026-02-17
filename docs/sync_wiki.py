#!/usr/bin/env python3
"""Sync docs/wiki to GitHub and GitLab wiki repos.

Requires the wiki repos to exist (create the first wiki page in the UI first).
"""

import argparse
import shutil
import subprocess
import sys
from pathlib import Path


def _cmd_text(cmd: list[str]) -> str:
    return " ".join(cmd)


def _hint_for_cmd(cmd: list[str], returncode: int) -> str | None:
    if not cmd or cmd[0] != "git":
        return None
    if cmd[:2] == ["git", "clone"]:
        return "Check the repo URL, network access, and credentials/token permissions."
    if cmd[:2] == ["git", "fetch"]:
        return "Verify remote access and auth. Try `git remote -v` inside the repo."
    if cmd[:2] == ["git", "pull"]:
        return "Local branch may have diverged. Resolve locally, then retry with a clean fast-forward."
    if cmd[:2] == ["git", "push"]:
        return "Verify push permissions and authentication for the wiki repository."
    if cmd[:2] == ["git", "commit"]:
        return "Configure `git config user.name` and `git config user.email` if commit identity is missing."
    return f"Run `{_cmd_text(cmd)}` manually for full git output."


def run(cmd, cwd=None):
    try:
        subprocess.run(cmd, cwd=cwd, check=True)
    except FileNotFoundError as e:
        raise SystemExit(
            f"Required command '{e.filename}' was not found while running: {_cmd_text(cmd)}. "
            "Install git and ensure it is on your PATH."
        ) from e
    except subprocess.CalledProcessError as e:
        hint = _hint_for_cmd(cmd, e.returncode)
        raise SystemExit(
            f"Command failed with exit code {e.returncode}: {_cmd_text(cmd)}"
            + (f". Hint: {hint}" if hint else "")
        ) from e


def git_output(cmd, cwd=None):
    try:
        return subprocess.check_output(cmd, cwd=cwd).decode("utf-8").strip()
    except FileNotFoundError as e:
        raise SystemExit(
            f"Required command '{e.filename}' was not found while running: {_cmd_text(cmd)}. "
            "Install git and ensure it is on your PATH."
        ) from e
    except subprocess.CalledProcessError as e:
        hint = _hint_for_cmd(cmd, e.returncode)
        raise SystemExit(
            f"Command failed with exit code {e.returncode}: {_cmd_text(cmd)}"
            + (f". Hint: {hint}" if hint else "")
        ) from e


def ensure_repo(url: str, repo_dir: Path):
    if not (repo_dir / ".git").is_dir():
        run(["git", "clone", url, str(repo_dir)])
        return

    run(["git", "fetch", "--all", "--prune"], cwd=repo_dir)
    branch = git_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=repo_dir)
    run(["git", "pull", "--ff-only", "origin", branch], cwd=repo_dir)


def clear_repo_worktree(repo_dir: Path):
    try:
        for child in repo_dir.iterdir():
            if child.name == ".git":
                continue
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()
    except OSError as e:
        raise SystemExit(
            f"Failed to clear repository worktree at {repo_dir}: {e}. "
            "Check file permissions and ensure files are not locked by another process."
        ) from e


def copy_source(source_dir: Path, repo_dir: Path):
    try:
        for item in source_dir.iterdir():
            dest = repo_dir / item.name
            if item.is_dir():
                shutil.copytree(item, dest)
            else:
                shutil.copy2(item, dest)
    except OSError as e:
        raise SystemExit(
            f"Failed to copy wiki files from {source_dir} to {repo_dir}: {e}. "
            "Check read/write permissions and available disk space."
        ) from e


def has_changes(repo_dir: Path) -> bool:
    status = git_output(["git", "status", "--porcelain"], cwd=repo_dir)
    return bool(status)


def commit_and_push(repo_dir: Path, message: str):
    run(["git", "add", "."], cwd=repo_dir)
    run(["git", "commit", "-m", message], cwd=repo_dir)
    run(["git", "push"], cwd=repo_dir)


def sync_one(name: str, url: str, source_dir: Path, workdir: Path, message: str):
    repo_dir = workdir / name
    ensure_repo(url, repo_dir)
    clear_repo_worktree(repo_dir)
    copy_source(source_dir, repo_dir)

    if has_changes(repo_dir):
        commit_and_push(repo_dir, message)
        print(f"[{name}] pushed")
    else:
        print(f"[{name}] no changes")


def main():
    parser = argparse.ArgumentParser(description="Sync docs/wiki to GitHub and GitLab wiki repos")
    parser.add_argument(
        "--source-dir",
        default=str(Path.cwd() / "docs" / "wiki"),
        help="Path to wiki source directory",
    )
    parser.add_argument(
        "--workdir",
        default="/tmp/sedsprintf_wiki_sync",
        help="Directory for cloning wiki repos",
    )
    parser.add_argument(
        "--github-url",
        default="https://github.com/Rylan-Meilutis/sedsprintf_rs.wiki.git",
        help="GitHub wiki repo URL",
    )
    parser.add_argument(
        "--gitlab-url",
        default="https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs.wiki.git",
        help="GitLab wiki repo URL",
    )
    parser.add_argument(
        "--commit-message",
        default="Sync wiki from docs/wiki",
        help="Commit message for wiki sync",
    )
    args = parser.parse_args()

    source_dir = Path(args.source_dir).resolve()
    workdir = Path(args.workdir).resolve()

    if not source_dir.is_dir():
        raise SystemExit(
            f"Source dir not found: {source_dir}. "
            "Pass `--source-dir` pointing to your local `docs/wiki` directory."
        )

    try:
        workdir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        raise SystemExit(
            f"Failed to create workdir {workdir}: {e}. "
            "Choose a writable directory with `--workdir`."
        ) from e

    sync_one("github_wiki", args.github_url, source_dir, workdir, args.commit_message)
    sync_one("gitlab_wiki", args.gitlab_url, source_dir, workdir, args.commit_message)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nInterrupted by user.", file=sys.stderr)
        raise SystemExit(130)
    except Exception as e:
        print(f"Error: Unexpected failure while syncing wiki: {e}", file=sys.stderr)
        raise SystemExit(1) from e
