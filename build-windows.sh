#!/usr/bin/env bash
#
# Cross-compile a self-contained Windows executable (nfcoffee.exe) from Linux.
#
# Requirements:
#   1. rustup target add x86_64-pc-windows-gnullvm
#   2. An LLVM-mingw toolchain (https://github.com/mstorsjo/llvm-mingw/releases)
#
# Usage:
#   MINGW=/path/to/llvm-mingw/bin ./build-windows.sh
#
# The resulting binary is fully self-contained (only Windows system DLLs) and
# runs without administrator rights; all state is stored next to the .exe.

set -euo pipefail
cd "$(dirname "$0")"

if [[ -z "${MINGW:-}" ]]; then
    echo "error: set MINGW to the llvm-mingw 'bin' directory, e.g." >&2
    echo "  MINGW=/opt/llvm-mingw/bin ./build-windows.sh" >&2
    exit 1
fi

if [[ ! -x "$MINGW/x86_64-w64-mingw32-clang" ]]; then
    echo "error: $MINGW/x86_64-w64-mingw32-clang not found" >&2
    exit 1
fi

export PATH="$MINGW:$PATH"
export CC_x86_64_pc_windows_gnullvm="$MINGW/x86_64-w64-mingw32-clang"
export AR_x86_64_pc_windows_gnullvm="$MINGW/x86_64-w64-mingw32-ar"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$MINGW/x86_64-w64-mingw32-clang"

# The gnullvm target links libunwind dynamically by default. Point the linker at
# a directory that contains ONLY the static archive so the .exe stays portable.
static_unwind="$(mktemp -d)"
trap 'rm -rf "$static_unwind"' EXIT
cp "$MINGW/../x86_64-w64-mingw32/lib/libunwind.a" "$static_unwind/"

export RUSTFLAGS="-C link-arg=-L$static_unwind"

cargo build --release --target x86_64-pc-windows-gnullvm

mkdir -p dist
cp target/x86_64-pc-windows-gnullvm/release/nfcoffee.exe dist/nfcoffee.exe
echo
echo "Built: dist/nfcoffee.exe"
"$MINGW/x86_64-w64-mingw32-objdump" -p dist/nfcoffee.exe | grep 'DLL Name' || true
