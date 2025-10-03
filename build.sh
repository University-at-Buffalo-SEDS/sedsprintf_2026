#!/bin/bash
set -e

BUILD_MODE=()
tests=""
BUILD_STM="OFF"


# Parse args in any order
for arg in "$@"; do
  case "$arg" in
    release)
      echo "Building release version."
      BUILD_MODE=(
      --release
      )
      ;;
    test)
          echo "Building release version."
          tests="test"
          ;;
    stm-build)
      echo "Configuring to save temporary object files and assembly output."
      BUILD_STM="ON"
      ;;
    *)
      echo "Unknown option: $arg"
      ;;
  esac
done


if [[ "$BUILD_STM" == "ON" ]]; then
  echo "Configuring to save temporary object files and assembly output."
  stm_build_args=(
    --no-default-features
    --target thumbv7em-none-eabihf
  )
else
  stm_build_args=()
fi

if [[ -n "$tests" ]]; then
  cargo test "${BUILD_MODE[@]}" "${stm_build_args[@]}"
else
cargo build "${BUILD_MODE[@]}" "${stm_build_args[@]}"
fi
