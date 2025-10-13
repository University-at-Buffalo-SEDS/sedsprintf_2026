cmake_minimum_required(VERSION 3.22)
project(sedsprintf_rs_cmake NONE)

# --- Where is the Rust crate root? (has Cargo.toml and C-Headers/)
# If this file lives in sedsprintf_rs/cmake/, the default is one directory up.
set(SEDSPRINTF_RS_DIR_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/")
set(SEDSPRINTF_RS_DIR "${SEDSPRINTF_RS_DIR_DEFAULT}" CACHE PATH "Path to sedsprintf_rs crate root")
get_filename_component(RUST_DIR "${SEDSPRINTF_RS_DIR}" REALPATH)

# --- Target triple (override if needed)
# --- Decide whether we're building for STM32 (embedded) or host
option(SEDSPRINTF_STM_BUILD "Build sedsprintf_rs for STM32 (thumbv7em)" OFF)
set(SEDSPRINTF_STM_BUILD OFF)
if (CMAKE_C_COMPILER MATCHES "arm-none-eabi"
        OR CMAKE_TOOLCHAIN_FILE MATCHES "stm32"
        OR DEFINED ENV{STM32CUBEIDE_DIR}
)
    set(SEDSPRINTF_STM_BUILD ON)
endif ()
# Derive a host Rust triple
set(HOST_RUST_TRIPLE "")

# Final Rust target triple
if (SEDSPRINTF_STM_BUILD)
    set(SEDSPRINTF_RS_TRIPLE "thumbv7em-none-eabihf/" CACHE STRING "Rust target triple" FORCE)
else ()
    set(SEDSPRINTF_RS_TRIPLE "${HOST_RUST_TRIPLE}" CACHE STRING "Rust target triple" FORCE)
endif ()
set(RUST_TRIPLE "${SEDSPRINTF_RS_TRIPLE}")

# --- Derived paths
set(RUST_PROFILE "$<IF:$<CONFIG:Debug>,debug,release>")
set(RS_LIB_DBG "${RUST_DIR}/target/${RUST_TRIPLE}debug/libsedsprintf_rs.a")
set(RS_LIB_REL "${RUST_DIR}/target/${RUST_TRIPLE}release/libsedsprintf_rs.a")
set(RS_INCLUDE "${RUST_DIR}/C-Headers")
set(RS_STAMP "${CMAKE_CURRENT_BINARY_DIR}/sedsprintf_rs.stamp")

# --- Validate paths up front (catch typos immediately)
if (NOT EXISTS "${RUST_DIR}/Cargo.toml")
    message(FATAL_ERROR "SEDSPRINTF_RS_DIR='${RUST_DIR}' does not contain Cargo.toml. "
            "Pass -DSEDSPRINTF_RS_DIR=/abs/path/to/sedsprintf_rs")
endif ()
if (NOT EXISTS "${RS_INCLUDE}")
    message(FATAL_ERROR "Expected headers at '${RS_INCLUDE}'. "
            "Does your crate expose C headers in C-Headers/?")
endif ()

# --- Track inputs
file(GLOB_RECURSE RS_SRC CONFIGURE_DEPENDS
        "${RUST_DIR}/src/*.rs"
        "${RUST_DIR}/build.rs"
        "${RUST_DIR}/Cargo.toml"
        "${RUST_DIR}/Cargo.lock"
)

# --- Build rule (produces the .a as BYPRODUCTS)
# Option + (optional) auto-detect


# Build script args (list → string)
set(RS_ARGS_LIST "")
if (SEDSPRINTF_STM_BUILD)
    list(APPEND RS_ARGS_LIST "stm-build")
endif ()
string(REPLACE ";" " " RS_ARGS_STR "${RS_ARGS_LIST}")

# Human-friendly comment
set(RS_COMMENT "Building sedsprintf_rs (${RUST_PROFILE}) for ${RUST_TRIPLE}")
if (SEDSPRINTF_STM_BUILD)
    set(RS_COMMENT "${RS_COMMENT} (STM32 mode)")
endif ()

add_custom_command(
        OUTPUT "${RS_STAMP}"
        BYPRODUCTS "${RS_LIB_DBG}" "${RS_LIB_REL}"
        COMMAND ${CMAKE_COMMAND} -E chdir "${RUST_DIR}" bash -lc "./build.sh ${RS_ARGS_STR}"
        COMMAND ${CMAKE_COMMAND} -E touch "${RS_STAMP}"
        DEPENDS ${RS_SRC}
        COMMENT "${RS_COMMENT}"
        VERBATIM
)

add_custom_target(sedsprintf_rs_build DEPENDS "${RS_STAMP}")

# --- Imported lib (what your main project links)
add_library(sedsprintf_rs STATIC IMPORTED GLOBAL)
set_target_properties(sedsprintf_rs PROPERTIES
        IMPORTED_LOCATION_DEBUG "${RS_LIB_DBG}"
        IMPORTED_LOCATION_RELEASE "${RS_LIB_REL}"
        INTERFACE_INCLUDE_DIRECTORIES "${RS_INCLUDE}"
)
add_dependencies(sedsprintf_rs sedsprintf_rs_build)
# --- Ensure final ELF has non-exec stack even if some objs miss .note.GNU-stack
# Create a tiny sentinel ASM source at configure time
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/noexecstack.S"
        "    .section .note.GNU-stack,\"\",%progbits\n")

# Interface helper that injects the sentinel + flags into any consumer
add_library(sedsprintf_rs_noexecstack INTERFACE)
# Make the sentinel compile into the final consumer target (requires parent to have ASM enabled)
target_sources(sedsprintf_rs_noexecstack INTERFACE
        "${CMAKE_CURRENT_BINARY_DIR}/noexecstack.S"
)
# Prevent assembler from marking stack executable; force linker header to noexec

include(CheckLinkerFlag)
include(CheckCCompilerFlag)
check_linker_flag(C "-Wl,-z,noexecstack" HAVE_LD_NOEXECSTACK)
check_c_compiler_flag("-Wa,--noexecstack" HAVE_AS_NOEXECSTACK)

if (HAVE_LD_NOEXECSTACK AND HAVE_AS_NOEXECSTACK)
    # (same INTERFACE target as above)
endif ()

# --- Make final ELF have non-exec stack where supported (Linux/*BSD)
if (SEDSPRINTF_STM_BUILD)  # ELF platforms only

    # These flags are understood by GCC/Clang toolchains on ELF
    target_compile_options(sedsprintf_rs INTERFACE -Wa,--noexecstack)
    target_link_options(sedsprintf_rs INTERFACE -Wl,-z,noexecstack)

    # When someone links sedsprintf_rs, also pull in the helper
else ()
    # macOS/Windows: nothing to do; Mach-O/PE don’t use -z,noexecstack.
endif ()


# Optional niceties
add_library(sedsprintf_rs::sedsprintf_rs ALIAS sedsprintf_rs)
set(SEDSPRINTF_RS_LIB_DEBUG "${RS_LIB_DBG}" PARENT_SCOPE)
set(SEDSPRINTF_RS_LIB_RELEASE "${RS_LIB_REL}" PARENT_SCOPE)
set(SEDSPRINTF_RS_INCLUDE_DIR "${RS_INCLUDE}" PARENT_SCOPE)

message(STATUS "sedsprintf_rs include dir: ${RS_INCLUDE}")
message(STATUS "sedsprintf_rs debug lib:  ${RS_LIB_DBG}")
message(STATUS "sedsprintf_rs release lib:${RS_LIB_REL}")
