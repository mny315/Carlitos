# Preserve cargo-xwin's SDK paths when cross-compiling from Linux.
if(DEFINED ENV{XWIN_CACHE_DIR})
  include("$ENV{XWIN_CACHE_DIR}/cmake/clang-cl/x86_64-pc-windows-msvc-toolchain.cmake")
else()
  set(CMAKE_SYSTEM_NAME Windows)
  set(CMAKE_SYSTEM_PROCESSOR AMD64)
endif()
set(OPUS_STATIC_RUNTIME ON CACHE BOOL "Link the same static CRT as Rust" FORCE)
