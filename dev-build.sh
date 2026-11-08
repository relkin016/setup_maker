#!/bin/bash
# Remote dev build: stubs (x64 + x86) and the CLI builder with embedded stubs.
set -e
cd ~/setup_maker
. ~/.cargo/env
# Stubs target Windows 7 SP1+ (tier-3 win7 targets: nightly + build-std).
for T in x86_64-win7-windows-gnu i686-win7-windows-gnu; do
  P=$( [ "${T%%-*}" = x86_64 ] && echo x86_64 || echo i686 )-w64-mingw32
  U=${T//-/_}
  env "CC_$U=$P-gcc" "CXX_$U=$P-g++" "AR_$U=$P-ar" \
    cargo +nightly build -q --release -p smk-stub --target "$T" -Zbuild-std=std,panic_abort
done
export SMK_STUB_X64=$PWD/target/x86_64-win7-windows-gnu/release/smk-stub.exe
export SMK_STUB_X86=$PWD/target/i686-win7-windows-gnu/release/smk-stub.exe
cargo build -q --release -p setupmaker "$@"
