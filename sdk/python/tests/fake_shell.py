"""A stand-in for the desktop's shell, for the tests: the three approval actions `yos` and the
dispatch call on `app-shell`, granting every request at once and spending each grant once — and
`agent.reach`, which a door asks about an agent token: a Reader held to `hello.look` at `safe`
(`tok-reader`), a live agent with no role (`tok-plain`), and no other live agent.

`support.ShellStandIn` runs this under a copy of the Python interpreter named `yantrik-ui`, so
the kernel's account of the listener (`SO_PEERCRED` → `/proc/<pid>/exe`) passes the shell-peer
rule exactly as the real shell does — nothing in the rule is patched. That the rule is only a
file name is the protocol's own caveat: anything running as the person can do this.
"""

from yantrik_surface import Refusal, Surface, reach

requests = {}
spent = []
AGENTS = {
    reach.token_digest("tok-reader"): (
        reach.HELD, reach.Reach("pi:c-read", "reader", "Reader", ["hello.look"], "safe")),
    reach.token_digest("tok-plain"): (reach.PLAIN, None),
}


def standing(digest):
    return AGENTS.get(digest, (reach.UNKNOWN, None))


class Shell(Surface):
    """Answers `agent.reach` beside `app.describe` and `app.act`, as the shell's dispatch does:
    from what it keeps, by the token's digest."""

    def handle_from(self, method, params, peer):
        if method == reach.ASK:
            kind, held = standing(params.get("token_sha256") if isinstance(params, dict) else None)
            answer = {"standing": kind}
            if held is not None:
                answer["reach"] = dict(held._asdict())
            return answer
        return super().handle_from(method, params, peer)


shell = Shell("shell", summary=lambda: "stand-in shell, %d granted" % len(requests),
              reach_of=lambda token: standing(reach.token_digest(token)))


@shell.view
def state():
    return {"requests": {k: list(v) for k, v in requests.items()}, "spent": list(spent)}


@shell.action("request_approval")
def request_approval(app: str, action: str, grade: str, args_json: dict,
                     requester: str = "") -> dict:
    """Put an approval card up."""
    request_id = "appr-%d" % (len(requests) + 1)
    requests[request_id] = (app, action, args_json)
    return {"status": "pending", "request_id": request_id}


@shell.action("approval_status")
def approval_status(request_id: str) -> dict:
    """Say what the person answered."""
    if request_id in spent:
        return {"status": "consumed"}
    return {"status": "granted" if request_id in requests else "unknown"}


@shell.action("consume_approval")
def consume_approval(request_id: str, app: str, action: str, args_json: dict) -> dict:
    """Spend a granted approval, once, for exactly the call it was granted for."""
    if request_id not in requests:
        raise Refusal("no approval request `%s`." % request_id)
    if request_id in spent:
        raise Refusal("`%s` was already used." % request_id)
    if tuple(requests[request_id]) != (app, action, args_json):
        raise Refusal("`%s` was approved for another call." % request_id)
    spent.append(request_id)
    return {"spent": request_id}


if __name__ == "__main__":
    shell.serve()
