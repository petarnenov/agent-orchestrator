# agent-orchestrator

CLI orchestrator for staged Copilot and Claude execution pipelines.

## What it does

`agent-orchestrator` reads a task from a `.md` or `.txt` file and runs a staged pipeline:

1. prospect1
2. prospect2
3. synthesis
4. implementation

At startup, the CLI first prompts you to choose an execution mode:

- `Plan only` - runs through synthesis, writes `plan.md` and `implementation.prompt.md` into the workspace root, and stops there
- `Full implementation` - runs all four phases and writes `plan.md` into the workspace root before continuing to implementation

It then prompts you to choose which agent to use for each stage (`Copilot` or `Claude`).

If the tool is started non-interactively, it falls back to `Full implementation` and the legacy default mapping:
`prospect1=Copilot`, `prospect2=Claude`, `synthesis=Claude`, `implementation=Copilot`.

Final deliverables are written into the workspace root:

- `plan.md` for both modes
- `implementation.prompt.md` only for `Plan only`

Run prompts, logs, prospect outputs, the implementation report, and `run-summary.json` are written into the generated run directory under `.agent-orchestrator-runs` unless `--output-dir` is set.

## Local usage

```bash
cargo run -- sample-task.md --working-dir .
```

Or with the compiled debug binary:

```bash
./target/debug/agent-orchestrator sample-task.md --working-dir .
```

Or after a release build:

```bash
cargo build --release
./target/release/agent-orchestrator sample-task.md --working-dir .
```

## Homebrew

After the first tagged release is published:

```bash
brew tap petarnenov/agent-orchestrator
brew install agent-orchestrator
```

## APT

After the first tagged release is published:

```bash
curl -fsSL https://petarnenov.github.io/agent-orchestrator/apt/public.key | \
  sudo gpg --dearmor -o /usr/share/keyrings/agent-orchestrator.gpg

echo "deb [signed-by=/usr/share/keyrings/agent-orchestrator.gpg] \
https://petarnenov.github.io/agent-orchestrator/apt stable main" | \
  sudo tee /etc/apt/sources.list.d/agent-orchestrator.list

sudo apt update
sudo apt install agent-orchestrator
```

## Release flow

Tagged releases (`v*`) trigger GitHub Actions that:

1. build macOS and Linux binaries
2. create `.tar.gz` archives
3. build a Debian package
4. attach all assets to GitHub Releases
5. publish an APT repository to GitHub Pages
6. update the Homebrew tap formula repository
