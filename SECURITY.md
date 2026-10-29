# Security policy

## Reporting a vulnerability

**Do not open a public issue.** Report it privately through GitHub's
[private vulnerability reporting](https://github.com/Londopy/EraSwitch/security/advisories/new).
If that doesn't work for you, email **Londopy@protonmail.com**.

You'll get an acknowledgement within 72 hours and an assessment within 7 days.

## Supported versions

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Anything older | ❌ (update through the in-app updater or the releases page) |

## Threat model

EraSwitch runs as your normal user, with no admin rights. It starts and stops
other programs (RetroBar, Open-Shell), writes their per-user settings, changes
your wallpaper and sound scheme, and keeps a snapshot of your
previous settings so it can put them back.

**In scope:**

- Anything that lets a crafted `.londopy` (or older `.londo`) era file write outside EraSwitch's own
  data folder, run a program, or read files it shouldn't
- The updater installing something that doesn't match the release's
  `SHA256SUMS.txt`
- EraSwitch launching a program from a path an attacker controls
- A way to make EraSwitch leave your desktop in a state it can't restore

**Out of scope:**

- Bugs in RetroBar, Open-Shell or Windows itself (report those upstream)
- Someone who already runs code as your user changing EraSwitch's config
- SmartScreen warnings: the binaries are unsigned (see below)

## Verifying downloads

Releases are not code-signed. Every release ships `SHA256SUMS.txt`; check a
download with:

```powershell
Get-FileHash .\EraSwitch-v0.1.0-x64-portable.exe -Algorithm SHA256
```

and compare it with the matching line in `SHA256SUMS.txt`.
