# Security Policy

## Supported versions

Only the latest release receives security fixes. Users should keep the hourly collector updated to the latest GitHub release.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability or leaked credential. Use GitHub's **Security → Advisories → Report a vulnerability** private reporting flow for this repository.

Include the affected version, operating system, reproduction steps, impact, and any suggested mitigation. We aim to acknowledge valid reports within three business days. We will coordinate disclosure after a fix is available.

## Security boundaries

The collector is designed to upload token counters and provider-reported utilization only. It must not upload prompts, responses, source code, project paths, AI-provider credentials, cookies, or Claude/Codex login tokens.

Release archives include SHA-256 checksums and GitHub artifact attestations. Verify a downloaded release with:

```bash
gh attestation verify worko-ai-usage-*.tar.gz -R revolution-uz/worko-ai-usage
sha256sum --check SHA256SUMS
```
