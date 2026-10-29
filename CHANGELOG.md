# Changelog

All notable changes to EraSwitch are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). The section for a version is what appears on its GitHub release page. Validated in CI by [patchnotes](https://github.com/Londopy/patchnotes).

## [Unreleased]

### Added

- A Windows Vista era: RetroBar's Vista Aero taskbar with the orb, and Open-Shell's Windows 7-style menu in its Aero skin.
- An Era menu in the tray and "Start from: Windows XP / Windows Vista" in Settings. Picking an era while another is on switches straight across, through Windows 11. Picking one keeps your wallpaper and sound choices.
- RetroBar's Vista themes (Aero, Basic, Classic, Longhorn Aero) in the taskbar theme list.

### Changed

- Eras are saved as `.londopy` files, the new name of the format all Londopy apps share. `.londo` eras from 0.1.x still open from Settings or when dropped on the exe.

## [0.1.1] - 2026-10-21

### Added

- The setup.exe has a "Helper programs" page: it finds out whether RetroBar and Open-Shell are installed and offers to install the missing ones with winget.
- EraSwitch offers the same install the first time it starts (for the MSI, portable and Scoop installs), and whenever you switch while a helper is missing, instead of only showing an error.
- After installing a helper, EraSwitch stops anything its installer started and turns off Open-Shell's autostart, so neither takes over the Windows 11 taskbar or Start menu until you switch.

### Fixed

- RetroBar is also found in a custom install folder (through its uninstall entry) and on ARM64, where winget installs it as a portable app behind a link.

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

[Unreleased]: https://github.com/Londopy/EraSwitch/compare/v0.1.1...HEAD
[0.1.0]: https://github.com/Londopy/EraSwitch/releases/tag/v0.1.0
[0.1.1]: https://github.com/Londopy/EraSwitch/compare/v0.1.0...v0.1.1
