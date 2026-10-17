# Changelog

All notable changes to EraSwitch are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The section for a version is
what appears on its GitHub release page. Validated in CI by
[patchnotes](https://github.com/Londopy/patchnotes).

## [Unreleased]

## [0.1.0] - 2026-10-17

### Added

- Switch between Windows 11 and a Windows XP look from the tray icon or with Ctrl+Alt+E.
- Taskbar through RetroBar (any of its XP themes) and Start menu through Open-Shell's XP Luna skin.
- Optional wallpaper and Windows sound scheme pieces.
- A snapshot of your Windows 11 settings, taken before every switch, that puts everything back.
- Recovery: a crash or Ctrl+C restores Windows 11, starting EraSwitch with the era still on resumes it (or finishes a restore that was cut off), and `EraSwitch.exe --restore` (also run by the uninstallers) undoes it from the command line.
- Exiting EraSwitch switches back to Windows 11 first, and stays running if that fails.
- Era files are checked before use: theme and skin names can't point outside RetroBar's and Open-Shell's own folders.
- Save and open eras as `.londo` files, wallpaper included.
- Finds RetroBar and Open-Shell, and installs them with winget from Settings.
- Run at sign-in, Windows notifications, and an in-app updater that checks SHA-256 and installs only when you click.

[Unreleased]: https://github.com/Londopy/EraSwitch/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Londopy/EraSwitch/releases/tag/v0.1.0
