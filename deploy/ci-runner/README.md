# Throwaway CI runners (#1234)

The `test` job of `ci.yml` runs on a self-hosted runner labelled `yantrik-build`. That runner
is not a machine: it is a container on node2 that is cloned for one job and destroyed when
the job ends. Nothing a job writes, plants or steals is there for the next one.

## Why it is built this way

GitHub's 16 GB runners lost the `test` job about half the time, with no log. Pranab decided
(15 Nov) to run it on node2. The first attempt was one long-lived runner on the build host
(CT 530); the security review of PR #1252 (comment 6888810312) rejected that, and this design
answers its findings.

| Review id | Finding | Answer here |
|---|---|---|
| C1 | A fork PR can edit `ci.yml` and send its own job to the build host | Fork-PR approval is `all_external_contributors` (set), and whatever runs on `yantrik-build` runs in a container that holds no credential, no cache and no neighbour worth reaching. The `runs-on` expression also excludes forks (L2) but is not the boundary. |
| H1 | Jobs share a uid and files, so one job can poison later ones and steal the runner's identity | A fresh clone per job. The toolchain (`/opt/rust`) and the runner (`/opt/actions-runner`) are root-owned; the job's user `runner` cannot write them. The runner's one-job (JIT) credentials are single use. A planted `.cargo/config.toml`, shim, registry file or target dir dies with the container. |
| H2 | Anyone with write access gets a shell on the build host | They get a shell in a container that is destroyed after the job. It is a separate machine in effect: its own CT, no host mounts, no secrets. The controller holds the token; the container never sees it. |
| M1 | The shared build lock can be held or broken | There is no lock: one runner at a time, one CT per job, and a memory gate (40 GB `MemAvailable`) in the controller instead. The 3 h hard limit bounds a stuck job. |
| M2 | Shared `/tmp` and host state | A private `/tmp` (its own CT). `fs.protected_symlinks=1`, `protected_hardlinks=1`, `protected_regular=2`, `protected_fifos=2` are set on the node by `make-template.sh` (they are host-wide; an unprivileged CT cannot set them). |
| L1 | Token left in `.git/config` | `persist-credentials: false` on the self-hosted path. |
| L2 | `head.repo.fork` | Added to the `runs-on` expression. |
| L3 | `rust-toolchain.toml` is under branch control | The toolchain is baked in; the job checks `rustc --version` is 1.92.0 first. |
| L4 | Stale comment | Deleted. |

## The pieces

- `make-template.sh` builds CT **9530** `yantrik-ci-template` and converts it to a template.
  Unprivileged, nesting on, 32 cores, 32 GB RAM, 8 GB swap, 60 GB on `local-lvm`, DHCP on
  `vmbr0`. It contains the build libraries from `ci.yml`, Rust 1.92.0 (root-owned in
  `/opt/rust`, with clippy and rustfmt), the latest GitHub Actions runner in
  `/opt/actions-runner` (root-owned, checksum-checked), the `runner` user (uid 1001, no
  password, no sudo), and the launcher `/usr/local/sbin/yantrik-ci-launch`. Before it becomes
  a template it proves `bwrap` works as `runner` and that `runner` cannot write `/opt/rust`
  or `/opt/actions-runner`.
  The listener writes its credential files and `run-helper.sh` into its own directory and
  finds that directory from where its binary really is, so a symlink farm does not work. The
  launcher therefore gives each job a copy (`/home/runner/actions-runner`, about 1-2 s) and
  leaves `/opt/actions-runner` pristine and root-owned.
- `yantrik-ci-controller.sh` (+ `.service`) runs one loop: wait for 40 GB of free memory,
  ask GitHub for a one-job runner config, clone CT 9530 to a free id in 9600-9699 (linked
  clone on `local-lvm`), start it, push the config into a root-only file that the launcher
  reads once and deletes, wait for the runner to exit (3 h limit), stop and destroy the CT,
  loop. It destroys any leftover CT in 9600-9699 when it starts. API failures and fast
  failures back off from 30 s, doubling to 10 min. It never touches an id outside the range.
- `install-controller.sh` copies the controller to `/usr/local/sbin/`, installs and enables
  the unit and does not start it while `/root/.yantrik-ci/token` is missing.

The token is only ever read from the file, and goes to GitHub on curl's stdin (`-H @-`):
never on a command line, in an environment variable, or in a log. Logs hold no token and no
JIT config.

## Operations

Run all of this as root on node2 (`192.168.4.152`).

**Install or update the controller**

    scp -r deploy/ci-runner root@192.168.4.152:/root/ci-runner
    ssh root@192.168.4.152 'bash /root/ci-runner/install-controller.sh'

**Rebuild the template** (new runner release, new toolchain, new packages). Stop the
controller first and wait for its CT to go; the script refuses while any clone exists.

    systemctl stop yantrik-ci-controller
    bash /root/ci-runner/make-template.sh
    systemctl start yantrik-ci-controller

A runner that is too old for GitHub tries to update itself and cannot (its directory is
root-owned); the job then fails fast and the controller backs off. Rebuilding the template
is the fix. Moving Rust is a PR of its own (`rust-toolchain.toml`), and must change
`RUST_VERSION` in `make-template.sh` and the `grep` in `ci.yml` with it.

**Rotate the token.** The token needs the Administration (write) permission on
`yantrikos/yantrik-os` (repository runners); it is a fine-grained PAT. Replace the file
without echoing it, then restart:

    install -d -m 700 /root/.yantrik-ci
    ( umask 077; cat > /root/.yantrik-ci/token )     # paste, then Ctrl-D
    chown root:root /root/.yantrik-ci/token; chmod 600 /root/.yantrik-ci/token
    systemctl restart yantrik-ci-controller

The controller re-reads the file for every job and refuses a file that is not root-owned
mode 600 or 400. Revoke the old token on GitHub.

**Stop CI from using the host** (the `test` job then waits for a runner; switch its
`runs-on` back to `ubuntu-24.04` if it must run meanwhile):

    systemctl disable --now yantrik-ci-controller

This stops the loop and destroys the current CT. `systemctl enable --now` brings it back.

**Read the logs**

    journalctl -u yantrik-ci-controller -f
    less /var/log/yantrik-ci/controller.log        # rotated weekly, 8 kept
    ls /var/log/yantrik-ci/runs/                   # each runner's own output

One line per event: `cycle`, `cloned`, `running`, and `done` with the CT id, the runner name,
start and end times and the job's exit code (124 means the 3 h limit). `pct list` shows the
CT of a job in flight (9600-9699); there should be none when idle.

## Known limits

- A runner waits for its job inside its CT; it is replaced after the 3 h limit, so a job
  that lands in the last seconds of a window can be cut. The runner is also deleted from
  GitHub when its CT ends.
- The JIT config is on the runner's command line for the life of the job, as `run.sh`
  takes it; it is good for one job only and is already spent when the job runs.
- A fork PR approved by a maintainer still runs on GitHub's hosted runner, never here.
- Run `ci.yml`'s hosted path unchanged; nothing in this directory touches it.
