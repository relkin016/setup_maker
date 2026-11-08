#!/bin/bash
# Remote dev build: stubs (x64 + x86) and the CLI builder with embedded stubs.
set -e
cd ~/setup_maker
. ~/.cargo/env
cargo build -q --release -p smk-stub --target x86_64-pc-windows-gnu
cargo build -q --release -p smk-stub --target i686-pc-windows-gnu
export SMK_STUB_X64=$PWD/target/x86_64-pc-windows-gnu/release/smk-stub.exe
export SMK_STUB_X86=$PWD/target/i686-pc-windows-gnu/release/smk-stub.exe
cargo build -q --release -p setupmaker "$@"
