# Changelog

## [Unreleased]

First release, planned as 0.0.1.

- Tray app for Windows and macOS: a speech bubble that shows dots while Claude
  Code works, turns green when it finishes, and turns amber when it's blocked
  on a permission prompt or a question.
- Notifications when Claude finishes or needs you. Turns under 30 seconds and
  prompts answered within 8 seconds stay quiet; both are adjustable.
- A menu listing every open session by project folder, most urgent first.
  Clicking a session brings its terminal to the front and marks it as seen.
- Connects to Claude Code on first launch by adding hooks to
  `~/.claude/settings.json`, keeping everything else in the file as it was.
  Toggle with "Connect to Claude Code", or `suavez install` / `uninstall`.
- `suavez status` prints what every session is doing.
- Run at login.
- Windows installer (Inno Setup), Chocolatey and winget manifests, a universal
  macOS DMG, and a Homebrew cask. Uninstalling through any of them removes the
  hooks.
- Landing page in `docs/`.
