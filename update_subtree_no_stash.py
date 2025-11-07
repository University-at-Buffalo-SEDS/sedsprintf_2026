#!/usr/bin/env python3
import subprocess


def run(cmd: list[str]) -> None:
    print("Running:", " ".join(cmd))
    subprocess.run(cmd, check=True)


def main() -> None:
    run([
        "git",
        "subtree",
        "pull",
        "--prefix=sedsprintf_rs",
        "sedsprintf-upstream",
        "main",
        "-m",
        "Merge sedsprintf_rs upstream main",
    ])


if __name__ == "__main__":
    main()
