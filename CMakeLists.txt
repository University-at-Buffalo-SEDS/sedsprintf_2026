cmake_minimum_required(VERSION 3.22)
project(sedsprintf_rs_cmake NONE)

# --- Where is the Rust crate root? (has Cargo.toml and C-Headers/)
# If this file lives in sedsprintf_rs/cmake/, the default is one directory up.
set(SEDSPRINTF_RS_DIR_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/")
set(SEDSPRINTF_RS_DIR "${SEDSPRINTF_RS_DIR_DEFAULT}" CACHE PATH "Path to sedsprintf_rs crate root")
get_filename_component(RUST_DIR "${SEDSPRINTF_RS_DIR}" REALPATH)

# --- Target triple (override if needed)
set(SEDSPRINTF_RS_TRIPLE "thumbv7em-none-eabihf" CACHE STRING "Rust target triple")
set(RUST_TRIPLE "${SEDSPRINTF_RS_TRIPLE}")

# --- Derived paths
set(RUST_PROFILE "$<IF:$<CONFIG:Debug>,debug,release>")
set(RS_LIB_DBG "${RUST_DIR}/target/${RUST_TRIPLE}/debug/libsedsprintf_rs.a")
set(RS_LIB_REL "${RUST_DIR}/target/${RUST_TRIPLE}/release/libsedsprintf_rs.a")
set(RS_INCLUDE "${RUST_DIR}/C-Headers")
set(RS_STAMP   "${CMAKE_CURRENT_BINARY_DIR}/sedsprintf_rs.stamp")

# --- Validate paths up front (catch typos immediately)
if(NOT EXISTS "${RUST_DIR}/Cargo.toml")
  message(FATAL_ERROR "SEDSPRINTF_RS_DIR='${RUST_DIR}' does not contain Cargo.toml. "
                      "Pass -DSEDSPRINTF_RS_DIR=/abs/path/to/sedsprintf_rs")
endif()
if(NOT EXISTS "${RS_INCLUDE}")
  message(FATAL_ERROR "Expected headers at '${RS_INCLUDE}'. "
                      "Does your crate expose C headers in C-Headers/?")
endif()

# --- Track inputs
file(GLOB_RECURSE RS_SRC CONFIGURE_DEPENDS
  "${RUST_DIR}/src/*.rs"
  "${RUST_DIR}/build.rs"
  "${RUST_DIR}/Cargo.toml"
  "${RUST_DIR}/Cargo.lock"
)

# --- Build rule (produces the .a as BYPRODUCTS)
add_custom_command(
  OUTPUT "${RS_STAMP}"
  BYPRODUCTS "${RS_LIB_DBG}" "${RS_LIB_REL}"
  COMMAND ${CMAKE_COMMAND} -E chdir "${RUST_DIR}" bash -lc "./build.sh ${RUST_PROFILE} stm-build"
  COMMAND ${CMAKE_COMMAND} -E touch "${RS_STAMP}"
  DEPENDS ${RS_SRC}
  COMMENT "Building sedsprintf_rs (${RUST_PROFILE}) for ${RUST_TRIPLE}"
  VERBATIM
)
add_custom_target(sedsprintf_rs_build DEPENDS "${RS_STAMP}")

# --- Imported lib (what your main project links)
add_library(sedsprintf_rs STATIC IMPORTED GLOBAL)
set_target_properties(sedsprintf_rs PROPERTIES
  IMPORTED_LOCATION_DEBUG   "${RS_LIB_DBG}"
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
target_compile_options(sedsprintf_rs_noexecstack INTERFACE -Wa,--noexecstack)
target_link_options(sedsprintf_rs_noexecstack INTERFACE -Wl,-z,noexecstack)

# When someone links sedsprintf_rs, also pull in the no-exec-stack helper
set_property(TARGET sedsprintf_rs APPEND PROPERTY
  INTERFACE_LINK_LIBRARIES sedsprintf_rs_noexecstack
)

# Optional niceties
add_library(sedsprintf_rs::sedsprintf_rs ALIAS sedsprintf_rs)
set(SEDSPRINTF_RS_LIB_DEBUG   "${RS_LIB_DBG}" PARENT_SCOPE)
set(SEDSPRINTF_RS_LIB_RELEASE "${RS_LIB_REL}" PARENT_SCOPE)
set(SEDSPRINTF_RS_INCLUDE_DIR "${RS_INCLUDE}" PARENT_SCOPE)

message(STATUS "sedsprintf_rs include dir: ${RS_INCLUDE}")
message(STATUS "sedsprintf_rs debug lib:  ${RS_LIB_DBG}")
message(STATUS "sedsprintf_rs release lib:${RS_LIB_REL}")
