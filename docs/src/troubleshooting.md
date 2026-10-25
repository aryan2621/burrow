# Troubleshooting

**"burrowd is not running and could not be started"** — run `burrowd` from a terminal to see the
error. Service logs: `~/.burrow/burrowd.log` (macOS), `journalctl --user -u burrowd` (Linux); on
Windows the service is the `Burrow\burrowd` scheduled task.

**`shell` says no sandbox runtime** — install one (see [Install](install.md)) and click **Check
again** in onboarding or restart the daemon. On macOS with Apple `container`, run
`container system start`.

**Model download stalls** — cancel and download again; it resumes where it stopped.

**llama-server won't start** — check `~/.burrow/llama-server.log`. Usually the model doesn't fit
in memory: pick a smaller one.

**An agent can't reach a site** — add the domain to its allowed domains, or approve it when asked.
Private network addresses must be listed exactly (e.g. `192.168.1.10`).

**Undo refused** — the file changed after the agent wrote it. Force the undo to overwrite your
change.

**The daemon crashed** — a report is in `~/.burrow/crashes/`. It stays on your machine; attach it
to an issue if you want help.

**Reset everything** — quit the app, stop the service, delete `~/.burrow/`.
