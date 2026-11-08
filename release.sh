#!/bin/bash
# Release build on the build server: stubs + Linux & Windows builders,
# packaged into dist/ with SHA256SUMS.
set -euo pipefail
cd "$(dirname "$0")"
. ~/.cargo/env
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
J=$(nproc)

cargo test -q -p smk-core --features encode
cargo test -q -p setupmaker --no-default-features
cargo build -q --release -j "$J" -p smk-stub --target x86_64-pc-windows-gnu
cargo build -q --release -j "$J" -p smk-stub --target i686-pc-windows-gnu
export SMK_STUB_X64=$PWD/target/x86_64-pc-windows-gnu/release/smk-stub.exe
export SMK_STUB_X86=$PWD/target/i686-pc-windows-gnu/release/smk-stub.exe
cargo build -q --release -j "$J" -p setupmaker
cargo build -q --release -j "$J" -p setupmaker --target x86_64-pc-windows-gnu

OUT=dist/setupmaker-$VER
mkdir -p "$OUT/linux/stubs" "$OUT/windows/stubs"
for d in linux windows; do
  cp "$SMK_STUB_X64" "$OUT/$d/stubs/smk-stub-x64.exe"
  cp "$SMK_STUB_X86" "$OUT/$d/stubs/smk-stub-x86.exe"
  cp -r examples README.md "$OUT/$d/"
done
cp target/release/setupmaker "$OUT/linux/"
cp target/x86_64-pc-windows-gnu/release/setupmaker.exe "$OUT/windows/"

cd dist
tar -C "setupmaker-$VER" -czf "setupmaker-$VER-linux-x64.tar.gz" linux
(cd "setupmaker-$VER" && zip -qr "../setupmaker-$VER-windows-x64.zip" windows)
sha256sum "setupmaker-$VER-linux-x64.tar.gz" "setupmaker-$VER-windows-x64.zip" > "SHA256SUMS-$VER.txt"
cat "SHA256SUMS-$VER.txt"
