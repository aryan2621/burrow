# Schedules

An agent's **Schedules** tab runs a prompt without you:

- **Interval:** every N seconds.
- **Cron:** a cron expression, e.g. `0 9 * * 1-5` for 9:00 on weekdays.
- **File:** when a file or folder inside the agent's folders changes (debounced 2 s).

Scheduled runs work in the background with the window closed, as long as the daemon runs (turn on
**Start at login**). Approvals still ask, as a notification. If the agent is busy, one firing
waits and later ones are skipped (and journaled). A time missed while the daemon was off fires once
at startup. On battery below 20%, scheduled runs wait until you plug in.
