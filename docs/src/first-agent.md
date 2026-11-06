# Your first agent

Onboarding walks through five steps: hardware check, model download (the recommended size is
preselected for your RAM, or a cloud key), sandbox check, start at login, and an **Inbox** agent:
it signs in to your email with an app password and writes a brief every morning (default 08:00),
moves newsletters and receipts to a `Later` folder and saves reply drafts. It never sends email.

An agent has:

- **Folders** it may read and write. Everything else on disk is off limits. It also gets a
  private scratch folder for temporary work.
- **A model**, built-in (`llama:<id>`) or cloud with your own key (`anthropic:<model>`, `openai:<model>`).
- **Allowed domains** for web access. Others ask you during a run.
- **Approval rules** per tool: `allow`, `ask` or `deny`.

Talk to it in **Chat**. Every action shows up in **Timeline**, where you can undo a single file
change or a whole run.
