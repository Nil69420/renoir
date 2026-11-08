#!/bin/bash -eu
cd $SRC/renoir/fuzz
cargo fuzz build -O --debug-assertions
for target in lean_corrupt_ring broadcast_corrupt_ring frame_corrupt_ring control_info sensor_validators; do
  cp target/x86_64-unknown-linux-gnu/release/$target $OUT/
done
