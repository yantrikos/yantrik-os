# Yantrik Mind

The first-party mind. It is not a harness of ours to install: the image carries it beside the OS
in `/opt/yantrik-mind`, and the first boot sets it up under its own account
(`yantrik-minds-firstboot.service` runs `yantrik-update migrate-minds`, #713 / PR #749), which
installs and starts `yantrik-mind.service` and its memory service.

Its memory is its own YantrikDB, `/var/lib/yantrik-mind/mind.db`, under the Mind's account: the
person cannot read that directory, and the Mind's unit cannot reach `/opt/yantrik/data`. Every
other mind gets YantrikDB too, a file of its own (#677).

Its model is never set by an install. "Use Yantrik models" on its row, with its card, is the one
way it is offered the gateway (yantrik-mind E.PROV2: loopback only, kept as a provider of its
own, never as its local private lane); until then it keeps the provider the person chose for it.

**Settings → Harnesses** lists it with *Install*, which runs `harnesses/lib/install/mind.sh`:
it checks the service is running and that the Mind has attached to the desktop, and records it
as installed only then. When the first boot did not set it up, the row says to run, once:

```sh
sudo yantrik-update migrate-minds
```

Once attached, it is offered for *Use* like any other mind (`yos act shell use_harness id=mind`).
