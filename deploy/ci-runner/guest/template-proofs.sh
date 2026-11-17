#!/bin/bash
# The proofs make-template.sh runs inside the build VM before it becomes a template (#1252). Run as root
# with SEALED=1 in a clone (not in the build VM), NODE_LAN_IP (the node's address on the LAN), NODE_BR_IP (the node on yci0) and LAN_IP (another
# LAN host) set. Prints PASS or FAIL per check; exits non-zero if any failed. It loads the VM's egress
# filter, as the launcher does, and removes it again at the end so the build can go on.
# shellcheck disable=SC2016,SC2329,SC2013
set -u
fail=0
check() { # check <label> <expected: ok|refused> <cmd...>
  local label=$1 want=$2 got
  shift 2
  if "$@" < /dev/null > /dev/null 2>&1; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then echo "PASS  $label ($got)"; else echo "FAIL  $label (wanted $want, got $got)"; fail=1; fi
}
as_runner() { runuser -u runner -- "$@"; }
trap 'nft delete table ip yantrik_ci > /dev/null 2>&1; nft delete table ip6 yantrik_ci > /dev/null 2>&1' EXIT

check "bwrap works as runner" ok as_runner bwrap --unshare-all --ro-bind / / --dev /dev --proc /proc true
check "runner cannot write /opt/rust" refused as_runner touch /opt/rust/x
check "runner cannot write /opt/rust/cargo/bin" refused as_runner touch /opt/rust/cargo/bin/x
check "runner cannot write /opt/actions-runner" refused as_runner touch /opt/actions-runner/x
check "runner cannot overwrite run.sh" refused as_runner sh -c ': >> /opt/actions-runner/run.sh'
check "runner is not root" ok sh -c '[ "$(runuser -u runner -- id -u)" = 1001 ]'
# The build VM still has sudo and sshd (seal.sh removes them after these proofs); a clone has neither.
if [ "${SEALED:-0}" = 1 ]; then
  check "sudo is not installed" refused command -v sudo
  check "no sshd is installed" refused command -v sshd
  check "no account but root, runner and the system's" ok sh -c '[ -z "$(getent passwd debian)" ]'
else
  echo "SKIP  sudo, sshd and the default account are removed after these proofs (SEALED=1 checks them in a clone)"
fi
check "runner has no sudo" refused as_runner sudo -n true
check "runner cannot su to root" refused as_runner su -c true root
check "runner cannot write the launcher" refused as_runner sh -c ': >> /usr/local/sbin/yantrik-ci-launch'
check "runner cannot write the job-started hook" refused as_runner sh -c ': >> /usr/local/libexec/yantrik-ci-job-started.sh'
check "runner cannot replace the hook" refused as_runner mv /usr/local/libexec/yantrik-ci-job-started.sh /usr/local/libexec/x
check "runner cannot write the egress rules" refused as_runner sh -c ': >> /etc/yantrik-ci/egress.nft'
check "runner cannot read the guest agent's channel" refused as_runner sh -c 'cat /dev/virtio-ports/org.qemu.guest_agent.0'

# The hook: it admits ci.yml's test job and nothing else.
hook=/usr/local/libexec/yantrik-ci-job-started.sh
ev=$(mktemp)
chmod 644 "$ev"
printf '{"pull_request":{"head":{"repo":{"full_name":"yantrikos/yantrik-os","fork":false}}}}' > "$ev"
hookrun() { as_runner env -i PATH=/usr/bin:/bin "$@" "$hook"; }
base="GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/ci.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_PATH=$ev"
# shellcheck disable=SC2086
check "hook admits push to main" ok hookrun $base GITHUB_EVENT_NAME=push
# shellcheck disable=SC2086
check "hook admits a same-repo PR" ok hookrun $base GITHUB_EVENT_NAME=pull_request
printf '{"pull_request":{"head":{"repo":{"full_name":"evil/yantrik-os","fork":true}}}}' > "$ev"
# shellcheck disable=SC2086
check "hook refuses a fork PR" refused hookrun $base GITHUB_EVENT_NAME=pull_request
check "hook refuses another workflow" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/x.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_NAME=push
check "hook refuses iso.yml (build, sign, publish stay on GitHub)" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/iso.yml@refs/heads/main GITHUB_JOB=test GITHUB_EVENT_NAME=push
check "hook refuses iso.yml's own job" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/iso.yml@refs/heads/main GITHUB_JOB=build GITHUB_EVENT_NAME=workflow_dispatch
# shellcheck disable=SC2086
check "hook refuses a scheduled run" refused hookrun $base GITHUB_EVENT_NAME=schedule
check "hook refuses another job" refused hookrun GITHUB_REPOSITORY=yantrikos/yantrik-os GITHUB_WORKFLOW_REF=yantrikos/yantrik-os/.github/workflows/ci.yml@refs/heads/main GITHUB_JOB=clippy GITHUB_EVENT_NAME=push
check "hook refuses another repository" refused hookrun GITHUB_REPOSITORY=other/repo GITHUB_JOB=test GITHUB_EVENT_NAME=push
rm -f "$ev"

# The egress filter, loaded the way the launcher loads it.
/usr/local/sbin/yantrik-ci-egress || { echo "FAIL  the egress filter did not load"; exit 1; }
check "no node API on the LAN (8006)" refused as_runner curl -m5 -sk "https://$NODE_LAN_IP:8006/"
check "no node ssh on the LAN" refused as_runner timeout 5 bash -c "exec 3<>/dev/tcp/$NODE_LAN_IP/22"
check "no node API on the bridge (8006)" refused as_runner curl -m5 -sk "https://$NODE_BR_IP:8006/"
check "no LAN host ssh" refused as_runner timeout 5 bash -c "exec 3<>/dev/tcp/$LAN_IP/22"
check "no LAN http" refused as_runner curl -m5 -s "http://$LAN_IP/"
check "no port 80 to the internet" refused as_runner curl -m5 -s http://example.com/
check "no ipv6" refused as_runner curl -m5 -6 -s https://api.github.com/zen
check "GitHub reachable" ok as_runner curl -m10 -sSfo /dev/null https://api.github.com/zen
check "index.crates.io reachable" ok as_runner curl -m10 -sSfo /dev/null https://index.crates.io/config.json
check "static.crates.io reachable" ok as_runner curl -m10 -sSfo /dev/null -r 0-0 https://static.crates.io/crates/libc/libc-0.2.150.crate
check "runner cannot list the rules" refused as_runner nft list ruleset
check "runner cannot flush the rules" refused as_runner nft flush ruleset
check "runner cannot reach the rules from a new namespace" refused as_runner unshare -Urn sh -c 'nft list ruleset && curl -m5 -s https://api.github.com/zen'
check "runner cannot flush the VM's rules from a user namespace" refused as_runner unshare -Ur nft flush ruleset
check "the VM's rules are still loaded after those attempts" ok sh -c 'nft list chain ip yantrik_ci output | grep -q "policy drop"'
# Our own public addresses are dropped: hit each, then the drop rule's counter must have moved.
for ip in $(grep -vE '^[[:space:]]*(#|$)' /etc/yantrik-ci/own-public-ips); do
  as_runner curl -m3 -sk "https://$ip/" > /dev/null 2>&1
done
check "own public addresses are dropped (counter moved)" ok sh -c 'nft list chain ip yantrik_ci output | grep -qE "ip daddr .* counter packets [1-9][0-9]* .* drop"'
# NoNewPrivileges, as the launcher sets it: no setuid binary raises runner; rootless bwrap still works.
nnp() { systemd-run --quiet --wait --pipe --collect --service-type=exec -p User=runner -p NoNewPrivileges=yes "$@"; }
check "the runner service has NoNewPrivs set" ok sh -c '[ "$(systemd-run --quiet --wait --pipe --collect -p User=runner -p NoNewPrivileges=yes sh -c "grep ^NoNewPrivs /proc/self/status | cut -f2")" = 1 ]'
check "rootless bwrap works under NoNewPrivileges" ok nnp bwrap --unshare-all --unshare-user --ro-bind / / --dev /dev --proc /proc true
rv=$(runuser -u runner -- bash -lc 'rustc --version' 2>&1 || true)
case "$rv" in "rustc 1.92.0 "*) echo "PASS  rustc is 1.92.0 for a login shell" ;; *) echo "FAIL  rustc: $rv"; fail=1 ;; esac
exit $fail
