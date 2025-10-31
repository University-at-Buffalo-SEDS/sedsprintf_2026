#!/bin/bash
set -e

BUILD_MODE=()
tests=""
BUILD_STM="OFF"
BUILD_PYTHON="OFF"
BUILD_WHEEL="OFF"
DEVELOP_WHEEL="OFF"


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
          echo "Building for STM32 target."
      BUILD_STM="ON"
      ;;
    python)
      echo "Building Python bindings."
      BUILD_PYTHON="ON"
      ;;
    maturin-build)
      echo "Building Python wheel."
      BUILD_WHEEL="ON"
      ;;
    maturin-develop)
      echo "Building and installing Python wheel in development mode."
      DEVELOP_WHEEL="ON"
      ;;
    *)
      echo "Unknown option: $arg"
      ;;
  esac
done


if [[ "$BUILD_STM" == "ON" ]]; then
  build_args=(
    --no-default-features
    --target thumbv7em-none-eabihf
    --features embedded
  )
elif [[ "$BUILD_PYTHON" == "ON" ]]; then
    build_args=(
        --features python
      )
else
  build_args=()
fi

if [[ -n "$tests" ]]; then
  cargo test "${BUILD_MODE[@]}" "${build_args[@]}"
elif [[ "$BUILD_WHEEL" == "ON" ]]; then
  maturin build "${BUILD_MODE[@]}"
elif [[ "$DEVELOP_WHEEL" == "ON" ]]; then
  maturin develop "${BUILD_MODE[@]}"
else
  cargo build "${BUILD_MODE[@]}" "${build_args[@]}"
fi
