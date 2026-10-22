<div align="center">

<img src="assets/icon-256.png" width="112" height="112" alt="Sua Vez">

# Sua Vez

A tray icon that lights up when your coding agent finishes, or is waiting on you.<br>
For Claude Code, Codex, Gemini CLI and Devin, on Windows and macOS.

<sub><i>sua vez</i>: Portuguese for <i>your turn</i></sub>

</div>

You give an agent a task, switch to something else, and come back twenty
minutes later. Then you find it stopped after thirty seconds to ask whether it
could run `npm install`. Sua Vez is a speech bubble in the tray that turns amber
when an agent is blocked on you and green when it's done, with a notification
for each.

It's a single 0.5 MB binary with no runtime dependencies. In the tray it uses
about 2 MB of memory and no measurable CPU.

## The bubble

| | |
|---|---|
| **Outline** | Nothing going on. |
| **Outline with dots** | An agent is working. |
| **Green** | An agent finished. Your turn. |
| **Amber with `!`** | An agent is blocked on a permission prompt or a question. |

With several sessions open, from one agent or several, the bubble shows the
most urgent one. Click it, with either button, to open the menu. On macOS the
quiet states follow the menu bar's tint, and the two that need you keep their
colour.

## The menu

Every open session is listed by project folder, most urgent first. Sessions
from agents other than Claude Code carry the agent's name:

```
2 need you · 1 your turn · 1 working
─────────────────────────────────────
⚠  cota wants to use Bash · 2m ago
⚠  porta (Codex) wants to use apply_patch · just now
●  cafe (Gemini): your turn · 5m ago
…  presence-sync is working · 1m
```

Click a session to bring its terminal to the front, whether that's Windows
Terminal, WezTerm, VS Code, iTerm or Terminal.app. Clicking also marks it as
seen, and a seen session stops colouring the bubble until it finishes again.

Sua Vez finds the app, not the tab. With several tabs in one Windows Terminal
window you get that window, and with two VS Code windows you get the one that
was in front last.

| | |
|---|---|
| **Mark all as seen** | Clears the colour from every finished or blocked session. |
| **Forget all sessions** | For a session stuck on "working" after a terminal crashed. Live sessions come back with their next event. |
| **Notify when it's your turn** | A notification when a turn ends. |
| **Notify when an agent needs you** | A notification when an agent is blocked on you. |
| **Quiet while its terminal is in front** | No notification while you're already in the session's terminal. On by default. |
| **Edit settings…** | Opens `config.json`. [See below.](#configuration) |
| **Connect to …** | One per installed agent. Adds or removes Sua Vez's hooks in that agent's settings. |
| **Run at login** | Per-user, no admin. |

### When it notifies

A notification for every reply would be noise when you're watching the
terminal anyway, so two rules keep it quiet:

- A turn that ends in under 30 seconds doesn't notify. You were almost
  certainly still looking.
- A permission prompt you answer within 8 seconds doesn't notify either.
- Nothing notifies while the session's terminal is the app in front. The bubble
  still turns, because Sua Vez can tell which app you're in but not which tab,
  and the finish may be in a tab you aren't looking at.

All three are adjustable. Sessions that finished before Sua Vez started still turn
the bubble green, but they don't send a burst of old notifications.

Running `suavez` again while it's already in the tray makes the running copy
tell you which sessions are waiting.

## Install

### Windows

```powershell
winget install kidchenko.SuaVez
# or
choco install suavez
```

Or grab [**SuaVez-Setup.exe**](https://github.com/kidchenko/suavez/releases/latest/download/SuaVez-Setup.exe)
directly. Each release also ships a portable `suavez-x.y.z-x64.exe` that runs
without installing; see [Releases](https://github.com/kidchenko/suavez/releases).

The binary isn't code-signed, so SmartScreen warns on first run: click **More
info → Run anyway**.

Use the installer if you want notifications to say "Sua Vez". Windows takes a
notification's name from a Start menu shortcut, and only the installer creates
one; a portable copy shows them under PowerShell's name instead. The installer
also offers to start Sua Vez when you sign in.

### macOS

```bash
brew install --cask kidchenko/tap/suavez
```

Or download [**SuaVez.dmg**](https://github.com/kidchenko/suavez/releases/latest/download/SuaVez.dmg)
(Apple silicon and Intel), open it, and drag Sua Vez into **Applications**. It
lives in the menu bar with no Dock icon.

The build isn't notarized, so Gatekeeper blocks the first launch from the DMG.
The Homebrew cask clears this for you. Otherwise, do it once, either with
**System Settings → Privacy & Security → Open Anyway** or with
`xattr -dr com.apple.quarantine /Applications/SuaVez.app`. Move the app into
Applications first, because the flag can't be cleared on the read-only disk
image.

### Connecting to your agents

Launch Sua Vez once after installing. A silent install through winget or
Chocolatey doesn't start it, so until you do (or until you next sign in, if
it's set to run at login) nothing is connected.

Sua Vez connects to each agent by itself the first time it finds it installed,
and tells you when it does. An agent you install later is picked up within 15
seconds, without a restart. If you disconnect one from the menu, it stays
disconnected.

| Agent | Where the hooks go |
|---|---|
| Claude Code | `~/.claude/settings.json`, or `$CLAUDE_CONFIG_DIR/settings.json` |
| Codex | `~/.codex/hooks.json`, or `$CODEX_HOME/hooks.json` |
| Gemini CLI | `~/.gemini/settings.json` |
| Devin | Nothing extra. Devin loads Claude Code's hooks by default, so connecting Claude Code connects Devin too. |

Agents read hooks when a session starts, so sessions that are already open
need a restart before they show up.

To do it from a terminal instead:

```
suavez install            # connect every installed agent
suavez install codex      # or just one: claude, codex or gemini
suavez uninstall          # disconnect from all of them
suavez status             # connections, and what every session is doing
```

Uninstalling through the Windows installer, winget, Chocolatey or Homebrew
removes the hooks from every agent too. If you delete a portable copy or drag
the app to the Trash, run `suavez uninstall` first, or untick each **Connect
to …**.

## How it works

Each agent can run a command on certain events and pass it a JSON description
on stdin: [Claude Code](https://code.claude.com/docs/en/hooks),
[Codex](https://learn.chatgpt.com/docs/hooks),
[Gemini CLI](https://geminicli.com/docs/hooks/reference.md) and
[Devin](https://docs.devin.ai/cli/extensibility/hooks/overview.md) all call
these hooks. Sua Vez registers itself, `suavez hook`, for the events that
matter:

| Becomes | Claude Code, Codex, Devin | Gemini CLI |
|---|---|---|
| ready | `SessionStart` | `SessionStart` |
| working, and the turn's clock starts | `UserPromptSubmit` | `BeforeAgent` |
| working (any permission prompt has been answered) | `PostToolUse` | `AfterTool` |
| needs you | `PermissionRequest`, `Notification` | `Notification` (tool permission) |
| your turn | `Stop` | `AfterAgent` |
| ready, without a notification (you pressed Esc) | `Interrupt` (Codex) | |
| the session is removed | `SessionEnd` | `SessionEnd` |

The hook process writes one small JSON file per session under `sessions/` in
the data folder, pokes the tray, and exits in about 50 ms. The first event of a
session takes about twice as long, because that's when it walks up its parent
processes to find the terminal app and saves it with the session. It always
exits 0, so a failure in Sua Vez never stops an agent. For Claude Code and Devin
it prints nothing; Codex and Gemini CLI insist on a JSON reply, so for them it
prints `{}`, which changes nothing. The tray rereads the folder when poked, plus
once every 15 seconds as a fallback and to drop sessions that have been quiet
for 8 hours. Those are usually ones whose terminal closed without a
`SessionEnd`.

When Sua Vez edits an agent's settings, it only adds or removes entries whose
command is its own `hook`. Your other hooks and settings stay as they were, in
the same order. If the file isn't valid JSON, it leaves it alone and tells you
why. If you move the binary, the next launch points the hooks at the new
location.

## Configuration

`%APPDATA%\SuaVez\config.json` on Windows,
`~/Library/Application Support/SuaVez/config.json` on macOS.

```json
{
  "notifyDone": true,
  "notifyNeedsYou": true,
  "minTurnSeconds": 30,
  "graceSeconds": 8,
  "quietWhenFocused": true,
  "staleHours": 8,
  "offered": ["claude", "codex"]
}
```

| Key | |
|---|---|
| `minTurnSeconds` | Turns shorter than this don't notify when they end. `0` notifies every time. |
| `graceSeconds` | How long a permission prompt waits before it notifies. |
| `quietWhenFocused` | Skip notifications while the session's terminal is in front. |
| `staleHours` | Sessions quiet for this long are dropped. |
| `offered` | Agents Sua Vez has already connected to by itself, so disconnecting one stays disconnected. |

Changes made by hand are picked up within 15 seconds. The log, `suavez.log`,
is in the same folder.

## Build

Rust 1.77 or newer.

```powershell
.\build.ps1           # Windows: tests, release exe, installer if Inno Setup is installed
.\build.ps1 -Icons    # ... and regenerate the icons
```

```bash
./build.sh            # macOS: tests, release binary, dist/SuaVez.app
./build.sh --dmg      # ... and a disk image
```

The icons are drawn in code (`src/icon.rs`). `suavez --dump-icons <dir>` writes
them as raw RGBA, and `assets/make_icons.py` turns those into `icon.ico` and
the PNGs.

## Releasing

1. Bump `version` in `Cargo.toml` and add a section to `CHANGELOG.md`.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`

The `release` workflow builds the DMG on macOS and the installer on Windows,
publishes both to a GitHub Release, and rewrites the Chocolatey, winget and
Homebrew manifests with the new version and checksums. Then:

- **Chocolatey** is pushed automatically when the `CHOCO_API_KEY` secret is set.
  New packages go through community moderation, so the first one takes a while.
- **winget**: submit the `manifest-winget` artifact to
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), or use
  `wingetcreate submit`.
- **Homebrew**: copy the `manifest-homebrew` artifact to `Casks/suavez.rb` in
  `kidchenko/homebrew-tap`.

Running the workflow by hand from the Actions tab is a dry run: everything is
built and uploaded as artifacts, and nothing is published.

## Not yet

- Bringing the exact tab forward, not just the terminal's window.
- Agents running inside WSL. Their hooks run on the Linux side and can't reach
  a Windows tray.
- Cloud agents, such as Devin sessions started in the browser or Slack. There's
  nothing running on your machine to call a hook.

## License

MIT
