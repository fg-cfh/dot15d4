#!/bin/sh -e

echo "TEST: build for ARM target"
cargo build --target=thumbv7em-none-eabihf -p dot15d4-frame3

echo "TEST: no features"
cargo test -p dot15d4-frame3 --no-default-features

echo "TEST: security"
cargo test -p dot15d4-frame3 --no-default-features --features=security

echo "TEST: ies"
cargo test -p dot15d4-frame3 --no-default-features --features=ies

echo "TEST: security,ies"
cargo test -p dot15d4-frame3 --no-default-features --features=security,ies
