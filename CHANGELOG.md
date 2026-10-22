# Changelog

## [Unreleased]

- No second notification for Claude Code sessions inside Claude Desktop, which
  already sends its own. The bubble still turns. Turn it off under
  **Settings → Quiet for Claude Desktop sessions**.
- On macOS 26 the menu no longer closes the first time you open Settings after
  launch, and it stays open while sessions update underneath it.

## [0.0.1] - 2026-10-22

First release.

- Tray app for Windows and macOS: a speech bubble that shows dots while a
  coding agent works, turns green when it finishes, and turns amber when it's
  blocked on a permission prompt or a question.
- Works with Claude Code, Codex, Gemini CLI and Devin. Sessions from all of
  them share one menu, labelled with the agent.
- Notifications when an agent finishes or needs you. Turns under 30 seconds and
  prompts answered within 8 seconds stay quiet, and so does anything from the
  terminal you're already in (in WezTerm, the pane you're in). All three are
  adjustable.
- A menu listing every open session by project folder, most urgent first.
  Clicking a session, or on Windows its notification, brings its terminal to
  the front and marks it as seen. In WezTerm it switches to the session's tab.
  Preferences sit one level down, under Settings.
- Connects to each agent the first time it finds it installed, by adding hooks
  to that agent's settings and keeping everything else in the file as it was.
  Devin rides on Claude Code's hooks. One "Connect to …" toggle per agent, or
  `suavez install [agent]` / `uninstall [agent]`.
  On macOS it waits until it runs from Applications rather than the disk
  image, so the hooks never point at a file that disappears on eject.
- `suavez status` prints the connections and what every session is doing.
- Run at login.
- Windows installer (Inno Setup), Chocolatey and winget manifests, a universal
  macOS DMG, and a Homebrew cask. Uninstalling through any of them removes the
  hooks.
- Landing page in `docs/`.
