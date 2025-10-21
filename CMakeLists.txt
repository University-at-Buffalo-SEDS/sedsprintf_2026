cmake_minimum_required(VERSION 3.22)
project(sedsprintf_rs_cmake NONE)

set(SEDSPRINTF_RS_DIR_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/")
set(SEDSPRINTF_RS_DIR "${SEDSPRINTF_RS_DIR_DEFAULT}" CACHE PATH "Path to sedsprintf_rs crate root")
get_filename_component(RUST_DIR "${SEDSPRINTF_RS_DIR}" REALPATH)

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
set(RS_ARGS_STR "")
# Final Rust target triple
if (SEDSPRINTF_STM_BUILD)
    set(SEDSPRINTF_RS_TRIPLE "thumbv7em-none-eabihf/" CACHE STRING "Rust target triple" FORCE)
    set(RS_ARGS_STR "${RS_ARGS_STR} stm-build")
else ()
    set(SEDSPRINTF_RS_TRIPLE "${HOST_RUST_TRIPLE}" CACHE STRING "Rust target triple" FORCE)
endif ()
set(RUST_TRIPLE "${SEDSPRINTF_RS_TRIPLE}")

# --- Derived paths
set(RUST_PROFILE "$<IF:$<CONFIG:Debug>,debug,release>")
set(RS_LIB_DBG "${RUST_DIR}/target/${RUST_TRIPLE}debug/libsedsprintf_rs.a")
set(RS_LIB_REL "${RUST_DIR}/target/${RUST_TRIPLE}release/libsedsprintf_rs.a")
set(RS_INCLUDE "${RUST_DIR}/C-Headers")

set(RS_ARG_REL " $<$<CONFIG:Release>:release>")

set(RS_ARGS_STR "${RS_ARGS_STR} ${RS_ARG_REL}")

if (NOT EXISTS "${RUST_DIR}/Cargo.toml")
    message(FATAL_ERROR "SEDSPRINTF_RS_DIR='${RUST_DIR}' does not contain Cargo.toml. "
            "Pass -DSEDSPRINTF_RS_DIR=/abs/path/to/sedsprintf_rs")
endif ()
if (NOT EXISTS "${RS_INCLUDE}")
    message(FATAL_ERROR "Expected headers at '${RS_INCLUDE}'. "
            "Does your crate expose C headers in C-Headers/?")
endif ()

# --- Track inputs ---
file(GLOB_RECURSE RS_SRC CONFIGURE_DEPENDS
        "${RUST_DIR}/src/*.rs"
        "${RUST_DIR}/build.rs"
        "${RUST_DIR}/Cargo.toml"
        "${RUST_DIR}/Cargo.lock"
        "${RUST_DIR}/C-Headers/*.h"
)

# --- Build rule: make the real artifacts be the OUTPUTs
add_custom_command(
        OUTPUT
        "${RS_LIB_DBG}" "${RS_LIB_REL}"
        COMMAND ${CMAKE_COMMAND} -E chdir "${RUST_DIR}" bash -lc "./build.sh ${RS_ARGS_STR}"
        # Optional: ensure the directories exist (helps with first build on some shells)
        DEPENDS ${RS_SRC}
        COMMENT "Building sedsprintf_rs (${RUST_PROFILE}) for ${RUST_TRIPLE}"
        USES_TERMINAL
        VERBATIM
)

# A target that depends on all produced files; any missing file will trigger rebuild
add_custom_target(sedsprintf_rs_build
        DEPENDS
        "${RS_LIB_DBG}" "${RS_LIB_REL}"
)

# --- Imported lib the C/C++ links against
add_library(sedsprintf_rs STATIC IMPORTED GLOBAL)

set_target_properties(sedsprintf_rs PROPERTIES
        IMPORTED_LOCATION_DEBUG "${RS_LIB_DBG}"
        IMPORTED_LOCATION_RELEASE "${RS_LIB_REL}"
        INTERFACE_INCLUDE_DIRECTORIES "${RS_INCLUDE}"
)
add_dependencies(sedsprintf_rs sedsprintf_rs_build)
# --- Ensure final ELF has non-exec stack even if some objs miss .note.GNU-stack

# Prevent assembler from marking stack executable; force linker header to noexecstack
include(CheckLinkerFlag)
include(CheckCCompilerFlag)

check_linker_flag(C "-Wl,-z,noexecstack" HAVE_LD_NOEXECSTACK)
check_c_compiler_flag("-Wa,--noexecstack" HAVE_AS_NOEXECSTACK)

# --- Make final ELF have non-exec stack where supported (Linux/*BSD)
if (SEDSPRINTF_STM_BUILD)
    # These flags are understood by GCC/Clang toolchains on ELF
    target_compile_options(sedsprintf_rs INTERFACE -Wa,--noexecstack)
    target_link_options(sedsprintf_rs INTERFACE -Wl,-z,noexecstack)
endif ()

add_compile_options(-ffunction-sections -fdata-sections -Os)
add_link_options(-Wl,--gc-sections -Wl,--icf=all)

# Optional niceties
add_library(sedsprintf_rs::sedsprintf_rs ALIAS sedsprintf_rs)
set(SEDSPRINTF_RS_LIB_DEBUG "${RS_LIB_DBG}" PARENT_SCOPE)
set(SEDSPRINTF_RS_LIB_RELEASE "${RS_LIB_REL}" PARENT_SCOPE)
set(SEDSPRINTF_RS_INCLUDE_DIR "${RS_INCLUDE}" PARENT_SCOPE)

message(STATUS "sedsprintf_rs include dir: ${RS_INCLUDE}")
message(STATUS "sedsprintf_rs debug lib:  ${RS_LIB_DBG}")
message(STATUS "sedsprintf_rs release lib:${RS_LIB_REL}")
