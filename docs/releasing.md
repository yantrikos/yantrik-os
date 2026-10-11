# Releasing Yantrik OS

A tag is a promise: this build passed a known set of checks on a real machine. This page says which
checks, and in what order.

## Channels

| Channel | Made by | When | Gate |
|---|---|---|---|
| **nightly** | `.github/workflows/iso.yml` | 06:00 UTC daily if `main` moved; any push to `iso/**`; by hand | CI green on `main`, the image boots in QEMU (`boottest.py`), and `release-check --tier ci` passes inside that boot |
| **beta** | a person, by tagging `vX.Y.Z-beta.N` | when a nightly has been lived on | everything nightly needs, plus `release-check --tier rc --interactive` on a real machine, plus the manual list |
| **stable** | a person, by tagging `vX.Y.Z` | when a beta has had no release-blocking issue for a week | the beta's gate again, on the exact commit being tagged |

A nightly is never tagged. Beta and stable tags are made by a person, never by a pipeline.

## `release-check`

Shipped in every bundle beside `yos` (`/opt/yantrik/bin/release-check`), and run on the machine
under test, as the person, in their session:

    release-check                       # tier ci: no network, no minds
    release-check --tier rc             # adds the minds and the browser
    release-check --tier rc --interactive   # adds the checks a person takes part in
    release-check --json report.json    # the same, written down
    release-check --may-lock            # also the check that locks the desktop (#203)

The lock check leaves the machine locked until a person signs in, and nothing on the socket can
undo that, so it runs only with `--may-lock`: the boot test's throwaway QEMU passes it, and a
person at the keyboard may. Never pass it on a shared test machine from afar.

**Tier ci** asserts, through the control surfaces:
- the shell answers and names its build; the mind mode is published for apps to read;
- every action on every surface declares a grade;
- every app opens and answers on its surface; every desktop screen shows when asked;
- a sensitive act without a grant is refused through `yos --no-ask` and through a raw socket call;
- the machine ceiling holds on a service (`system-monitor.kill_process` does not kill);
- a notification is filed under who sent it;
- the Agents workspace is wired; the bundle carries its changelog;
- the shell is idle when idle (CPU and memory bounds), and nothing crashed during the run;
- last, in every tier: no launch failed, no window is listed twice, and no window of the run's own
  is left open.

**Tier rc** adds every attached mind answering a one-word request, and the browser being drivable
through `yos web` — in a tab of the run's own (`YOS_WEB_TAB`), closed after; the browser too when
the run started it, and never a tab or window the person had open. With `--interactive` it also
puts up an approval card for the person to allow, and has pi run `exit 3` through its own terminal
to see the exit code arrive in its pane.

Every check prints what it saw. PASS and SKIP (with its reason) do not fail a run; FAIL does.

## Cutting a beta

1. Pick the nightly to promote; note its commit.
2. Install that nightly (or deploy that commit) on a real machine with minds attached.
3. Run `release-check --tier rc --interactive --json rc.json` there. Every check PASS or SKIP.
4. Walk the manual list it prints. Anything that fails is an issue labelled `release-blocker`.
5. No open `release-blocker`: tag `vX.Y.Z-beta.N` on that commit, attach `rc.json` to the release,
   and publish to the beta channel with the ISO workflow's `beta` option.

## When a check is wrong

A check that fails on a good build is a bug in the check: fix it in the same PR that proves the
build is good, and say so. A check is never skipped to get a tag out.
