#!/usr/bin/env python3
"""Sync docs/wiki to GitHub and GitLab wiki repos.

Requires the wiki repos to exist (create the first wiki page in the UI first).
"""

import argparse
import shutil
import subprocess
from pathlib import Path


def run(cmd, cwd=None):
    subprocess.run(cmd, cwd=cwd, check=True)


def git_output(cmd, cwd=None):
    return subprocess.check_output(cmd, cwd=cwd).decode("utf-8").strip()


def ensure_repo(url: str, repo_dir: Path):
    if not (repo_dir / ".git").is_dir():
        run(["git", "clone", url, str(repo_dir)])
        return

    run(["git", "fetch", "--all", "--prune"], cwd=repo_dir)
    branch = git_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=repo_dir)
    run(["git", "pull", "--ff-only", "origin", branch], cwd=repo_dir)


def clear_repo_worktree(repo_dir: Path):
    for child in repo_dir.iterdir():
        if child.name == ".git":
            continue
        if child.is_dir():
            shutil.rmtree(child)
        else:
            child.unlink()


def copy_source(source_dir: Path, repo_dir: Path):
    for item in source_dir.iterdir():
        dest = repo_dir / item.name
        if item.is_dir():
            shutil.copytree(item, dest)
        else:
            shutil.copy2(item, dest)


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
        raise SystemExit(f"Source dir not found: {source_dir}")

    workdir.mkdir(parents=True, exist_ok=True)

    sync_one("github_wiki", args.github_url, source_dir, workdir, args.commit_message)
    sync_one("gitlab_wiki", args.gitlab_url, source_dir, workdir, args.commit_message)


if __name__ == "__main__":
    main()
