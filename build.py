#!/usr/bin/env python3
import sys
import subprocess


def main(argv: list[str]) -> None:
  build_mode: list[str] = []
  tests = False
  build_stm = False
  build_python = False
  build_wheel = False
  develop_wheel = False

  # Parse args in any order
  for arg in argv:
    if arg == "release":
      print("Building release version.")
      build_mode = ["--release"]
    elif arg == "test":
      print("Running tests.")
      tests = True
    elif arg == "stm-build":
      print("Building for STM32 target.")
      build_stm = True
    elif arg == "python":
      print("Building Python bindings.")
      build_python = True
    elif arg == "maturin-build":
      print("Building Python wheel.")
      build_wheel = True
    elif arg == "maturin-develop":
      print("Building and installing Python wheel in development mode.")
      develop_wheel = True
    else:
      print(f"Unknown option: {arg}")

  # Decide build args
  if build_stm:
    build_args = [
      "--no-default-features",
      "--target",
      "thumbv7em-none-eabihf",
      "--features",
      "embedded",
    ]
  elif build_python:
    build_args = [
      "--features",
      "python",
    ]
  else:
    build_args = []

  # Dispatch to the correct command
  if tests:
    cmd = ["cargo", "test", *build_mode, *build_args]
  elif build_wheel:
    cmd = ["maturin", "build", *build_mode]
  elif develop_wheel:
    cmd = ["maturin", "develop", *build_mode]
  else:
    cmd = ["cargo", "build", *build_mode, *build_args]

  print("Running:", " ".join(cmd))
  subprocess.run(cmd, check=True)


if __name__ == "__main__":
  main(sys.argv[1:])
