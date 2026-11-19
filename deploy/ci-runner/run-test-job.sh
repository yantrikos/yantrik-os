#!/bin/bash
# What ci.yml's `test` job runs on the throwaway VM (deploy/ci-runner/): check the baked-in
# compiler, then build and test the workspace in the two memory phases below. It runs on the VM
# as the job, after the VM's job-started hook (guest/yantrik-ci-job-started.sh) has admitted the
# job; it is not a boundary, and the branch it comes from may edit it like any file in the job.
#
# The VM has the toolchain, the native libraries and bubblewrap baked in, root-owned, and the
# workshops root made. It is fresh for this one job, so there is no lock to take, no shared
# target dir and no temp dir to clean. The compiler is checked rather than trusted: a branch
# may edit rust-toolchain.toml.
set -euo pipefail
cd "$(dirname "$0")/../.."

rustc --version | grep -q '^rustc 1.92.0 '

# (the OOM priority is set by the controller on the VM's QEMU process on the node, not inside the guest)
# Memory, measured on the node (gate 5, 18 Nov; the serial console of the VM, which has 24 GB and no
# swap, and shares a node that cannot give it more): one rustc of the generated UI crate holds about 15.7 GB resident, and it is compiled twice, as
# the library and as the library's test harness; any other rustc held about 3.5 GB. At -j 24 and at
# -j 8 two of the big ones ran together beside the rest and the kernel killed one, which ended the runner.
# So two phases, and the big compile never overlaps anything:
#   1. everything except the UI crate and the one crate that depends on it, at -j 4:
#      4 x 3.5 GB = 14 GB, plus the OS, the runner and its 2 GB tmpfs: under 20 GB;
#   2. the rest (the UI crate's two units and its dependent) one job at a time: about 16 GB at most, so the job as a whole stays under 22 GB.
# Both use the configuration the tests use (the Build step in .github/actions/hosted-setup/action.yml says why), so the test
# step finds everything built. Tests run at -j 4 as well.
cargo build --workspace --exclude yantrik-ui --exclude yantrik-ui-slint --locked --lib --bins --tests --examples -j 4
cargo build --workspace --locked --lib --bins --tests --examples -j 1
cargo test --workspace --locked -j 4 --no-fail-fast
