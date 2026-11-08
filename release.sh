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

# Windows 7 editor (import/build): Rust's win7 targets are tier 3 -> nightly + build-std.
# Stubs stay on the regular targets (the installer itself requires Windows 10+).
rustup component add rust-src --toolchain nightly >/dev/null 2>&1 || true
for T in x86_64-win7-windows-gnu i686-win7-windows-gnu; do
  P=$( [ "${T%%-*}" = x86_64 ] && echo x86_64 || echo i686 )-w64-mingw32
  U=${T//-/_}
  env "CC_$U=$P-gcc" "CXX_$U=$P-g++" "AR_$U=$P-ar" \
    cargo +nightly build -q --release -j "$J" -p setupmaker --target "$T" -Zbuild-std=std,panic_abort
done

OUT=dist/setupmaker-$VER
mkdir -p "$OUT/linux/stubs" "$OUT/windows/stubs" "$OUT/windows7-x64/stubs" "$OUT/windows7-x86/stubs"
for d in linux windows windows7-x64 windows7-x86; do
  cp "$SMK_STUB_X64" "$OUT/$d/stubs/smk-stub-x64.exe"
  cp "$SMK_STUB_X86" "$OUT/$d/stubs/smk-stub-x86.exe"
  cp -r examples README.md "$OUT/$d/"
done
cp target/release/setupmaker "$OUT/linux/"
cp target/x86_64-pc-windows-gnu/release/setupmaker.exe "$OUT/windows/"
cp target/x86_64-win7-windows-gnu/release/setupmaker.exe "$OUT/windows7-x64/"
cp target/i686-win7-windows-gnu/release/setupmaker.exe "$OUT/windows7-x86/"

cd dist
tar -C "setupmaker-$VER" -czf "setupmaker-$VER-linux-x64.tar.gz" linux
(cd "setupmaker-$VER" && zip -qr "../setupmaker-$VER-windows-x64.zip" windows)
for a in x64 x86; do
  (cd "setupmaker-$VER" && zip -qr "../setupmaker-$VER-windows7-$a.zip" "windows7-$a")
done
sha256sum "setupmaker-$VER-linux-x64.tar.gz" "setupmaker-$VER-windows-x64.zip" \
  "setupmaker-$VER-windows7-x64.zip" "setupmaker-$VER-windows7-x86.zip" > "SHA256SUMS-$VER.txt"
cat "SHA256SUMS-$VER.txt"
