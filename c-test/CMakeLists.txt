cmake_minimum_required(VERSION 3.18)
project(c_test C)

set(CMAKE_C_STANDARD 11)

# --- Paths (adjust if your layout differs) ---
# Directory that contains your C header (e.g., sedsprintf_c.h)
set(SEDSPRINTF_C_HEADERS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../C-Headers)

# Prebuilt Rust static lib (host/macOS build shown)
set(SEDSPRINTF_LIB ${CMAKE_CURRENT_SOURCE_DIR}/../target/release/libsedsprintf_rs.a)
# For STM32 instead, use:
# set(SEDSPRINTF_LIB ${CMAKE_CURRENT_SOURCE_DIR}/../target/thumbv7em-none-eabihf/release/libsedsprintf_rs.a)

# Helpful check
if(NOT EXISTS "${SEDSPRINTF_LIB}")
    message(FATAL_ERROR "Rust static library not found at: ${SEDSPRINTF_LIB}
Build it first, e.g.: cargo build --release")
endif()

# --- Your C executable ---
add_executable(c_test main.c)
target_include_directories(c_test PRIVATE ${SEDSPRINTF_C_HEADERS_DIR})

# --- Import the Rust static library and link it ---
add_library(sedsprintf_rs STATIC IMPORTED GLOBAL)
set_target_properties(sedsprintf_rs PROPERTIES
        IMPORTED_LOCATION "${SEDSPRINTF_LIB}"
        INTERFACE_INCLUDE_DIRECTORIES "${SEDSPRINTF_C_HEADERS_DIR}"
)

target_link_libraries(c_test PRIVATE sedsprintf_rs)
