# The OS refuses a Mind bundle that lacks a capability the OS build relies on

#566 (merged 12 Oct) makes yos-mcp's command answers start with a bridge-written first line. A Yantrik Mind that predates E.CMD1 misreads that line, so failed commands read as done. So the OS must never pair this bridge with such a Mind, on a fresh ISO install or on a Mind reinstall.

## The contract (yantrik-mind-3e, E.CMD1b, `os-shared-memory` fc5aa7b)
- The Mind binary carries one compiled-in marker string, **exactly** `ym-capabilities: agent_token cmd_answer;`, ending at `;`. A Rust string has no terminator in the binary, so without the `;` any lowercase bytes after it would read as extra words.
  - Read it with `grep -aoE 'ym-capabilities: [a-z_ ]*;'`, take the first match with `sed -n 1p` (**not `head`**: under `pipefail`, head closing the pipe SIGPIPEs grep and fails a binary that has the marker), and split the words before the `;`.
  - `yantrik-mind --capabilities` prints the marker, but install-mind never runs the binary, so don't require it.
- The first bundle with it: `yantrik-mind-fc5aa7b-linux-amd64.tar.zst`, sha256 `f4784c51c41d659f7a11b274d3aafba0b20cda30f18ac0d80e068379247c5b64` (BUILD: `commit=fc5aa7b`, `agent_token=1`, `cmd_answer=1`). It also carries E.PRIV1b, so it is the bundle the consent mirror needs. It is not installed anywhere yet; mind-3e asks Pranab first.
- Use that bundle as the test fixture that must be accepted, and a 6a6006c-style BUILD (no `cmd_answer`) as one that must be refused.
- The Mind's `package.sh` writes `agent_token=1` and `cmd_answer=1` lines into the bundle's `BUILD` only when the binary's bytes contain the capability. A new script packaging an old binary therefore cannot claim a capability.
- Bundles from 6a6006c and earlier have no `cmd_answer` line. Wait for the first bundle that does before shipping this; mind-3e sends its commit.

## The change
Use one shared check, not two copies: a small function in `deploy/yantrik-os/` sourced by both places.
1. `build-debian-iso.sh` (around line 746, beside the existing `agent_token=1` refusal): refuse a `MIND_TARBALL` whose `BUILD` lacks `cmd_answer=1`, or whose binary (`bin/yantrik-mind`) does not contain both the bytes `ym-capabilities:` and the word `cmd_answer` on that marker line. Do the same for `agent_token`, which closes its existing weakness. Name the missing capability and the fix in the message.
2. `yantrik-update install-mind`: the same check before anything is put in place. A refused bundle leaves the installed Mind untouched.
3. The required capability list lives in one place, so a future capability is a one-line addition.

## Tests
`yantrik-update`'s selftest and a shell test of the ISO check, using fake bundles:
- `BUILD` with both lines and a binary carrying the marker: accepted.
- `BUILD` without `cmd_answer=1`: refused.
- `BUILD` with `cmd_answer=1` but no marker in the binary (a hand-edited BUILD): refused.
- Marker present but without `cmd_answer`: refused.

## Ordering
Merge only after mind-3e's first bundle with `cmd_answer=1` exists, and install that bundle on 520 and the Mac first. Otherwise reinstalling their current 6a6006c Mind would be refused, which is correct but inconvenient. It touches install-mind (the HELD path): send the head to yantrik-mind-3e before 520.
