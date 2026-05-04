# Security policy

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

Instead, email **hello@elevenways.be** with:

- A description of the issue
- Steps to reproduce
- Affected versions (if known)
- Your assessment of impact

We aim to acknowledge reports within 5 working days and provide a remediation plan within 30 days for confirmed issues. Coordinated disclosure is preferred — we will agree on a public disclosure date once a fix is available.

## Supported versions

dpub is in early development (pre-1.0). Until 1.0, only the latest released version receives security fixes.

## Scope

In scope:

- Crashes, panics, or undefined behaviour triggered by malformed DAISY or EPUB input
- Path-traversal, zip-slip, or similar issues during EPUB extraction or assembly
- Resource exhaustion (memory, CPU) on adversarially crafted but well-formed input

Out of scope:

- Issues in dependencies that have not yet been disclosed upstream (please report to upstream first)
- Issues that require an attacker to already have full local access to the user's machine
