# Worko AI Usage

Privacy-safe, open-source Claude Code and OpenAI Codex usage collector for Worko HR. It reads token counters from local session logs and uploads hourly aggregates. Prompts, source code, project paths, Claude/Codex API keys, cookies, and provider login tokens are never uploaded.

Every release contains SHA-256 checksums, an SPDX 2.3 SBOM, and GitHub/Sigstore build provenance. The installers verify the selected binary archive against the published checksum before installation.

## Install

### macOS and Linux

```bash
curl -fsSL https://github.com/revolution-uz/worko-ai-usage/releases/latest/download/install.sh | bash
```

Optional non-interactive URL/email arguments still prompt securely for the password:

```bash
curl -fsSL https://github.com/revolution-uz/worko-ai-usage/releases/latest/download/install.sh | bash -s -- --url https://hr.example.com --email employee@example.com
```

The installer detects Intel/Apple Silicon or x86_64/ARM64, installs to `~/.local/bin`, configures an hourly launchd/systemd job, and opens account login.

### Windows 10/11

Run PowerShell as the current user:

```powershell
irm https://github.com/revolution-uz/worko-ai-usage/releases/latest/download/install.ps1 | iex
```

The installer detects x64/ARM64, installs under `%LOCALAPPDATA%\WorkoAiUsage`, adds it to the user PATH, configures a Task Scheduler job, and opens account login. Administrator access is not required.

## Commands

```text
worko-ai-usage login [--url URL] [--email EMAIL]
worko-ai-usage status
worko-ai-usage sync
worko-ai-usage logout
```

- `login` stores only a Worko HR access token in the operating system's user config directory.
- `status` prints locally detected counters and does not contact Worko.
- `sync` uploads at most the latest 48 hourly snapshots.
- `logout` deletes the local Worko token and does not affect Claude or Codex login.

## Data collected

| Field | Purpose |
|---|---|
| Provider (`claude`/`codex`) | Separate agent reporting |
| Anonymous machine hash | Avoid duplicate hourly snapshots |
| UTC hour | Hourly reporting |
| Input/cached/output token counts | Usage KPI |
| Usage event count | Activity indicator |
| Provider-reported 5-hour percentage | Included only when present in local provider logs |

## Development and releases

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- status
```

Pushing a `v*` tag builds GitHub Actions release binaries for macOS, Linux, and Windows on x86_64 and ARM64. Installers always download the latest published release.

### Security gates

- Locked, reproducible Cargo dependency graph
- Formatting, Clippy with warnings denied, and tests on Linux, macOS, and Windows
- CodeQL Rust SAST with extended security queries
- RustSec advisory audit and `cargo-deny` license/source policy
- Pull-request dependency review at moderate severity or higher
- OpenSSF Scorecard and SARIF upload
- Dependabot for Cargo and GitHub Actions
- External Actions pinned to immutable full commit SHAs
- Least-privilege workflow permissions
- SHA-256 manifests, SPDX SBOM, and build provenance attestations

Verify a release archive independently:

```bash
gh attestation verify worko-ai-usage-*.tar.gz -R revolution-uz/worko-ai-usage
sha256sum --check SHA256SUMS
```

Repository administrators should additionally enable private vulnerability reporting, secret scanning with push protection, immutable releases, required signed commits, and branch protection requiring the CI and Security checks. These controls live in GitHub repository settings and cannot be enforced by workflow YAML alone.

## License

MIT
