# Changelog

## [Unreleased]

First release, planned as 0.0.1.

- Tray app for Windows and macOS: a speech bubble that shows dots while a
  coding agent works, turns green when it finishes, and turns amber when it's
  blocked on a permission prompt or a question.
- Works with Claude Code, Codex, Gemini CLI and Devin. Sessions from all of
  them share one menu, labelled with the agent.
- Notifications when an agent finishes or needs you. Turns under 30 seconds and
  prompts answered within 8 seconds stay quiet, and so does anything from the
  terminal you're already in. All three are adjustable.
- A menu listing every open session by project folder, most urgent first.
  Clicking a session brings its terminal to the front and marks it as seen.
- Connects to each agent the first time it finds it installed, by adding hooks
  to that agent's settings and keeping everything else in the file as it was.
  Devin rides on Claude Code's hooks. One "Connect to …" toggle per agent, or
  `suavez install [agent]` / `uninstall [agent]`.
- `suavez status` prints the connections and what every session is doing.
- Run at login.
- Windows installer (Inno Setup), Chocolatey and winget manifests, a universal
  macOS DMG, and a Homebrew cask. Uninstalling through any of them removes the
  hooks.
- Landing page in `docs/`.
