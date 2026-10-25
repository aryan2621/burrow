# Your first agent

Onboarding walks through five steps: hardware check, model download (the recommended size is
preselected for your RAM), sandbox check, start at login, and a first agent from the
**Research and documents assistant** template.

An agent has:

- **Folders** it may read and write. Everything else on disk is off limits. It also gets a
  private scratch folder for temporary work.
- **A model**, built-in (`llama:<id>`) or Ollama (`ollama:<name>`).
- **Allowed domains** for web access. Others ask you during a run.
- **Approval rules** per tool: `allow`, `ask` or `deny`.

Talk to it in **Chat**. Every action shows up in **Timeline**, where you can undo a single file
change or a whole run.
