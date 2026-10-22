<div align="center">

<img src="assets/icon-256.png" width="112" height="112" alt="Sua Vez">

# Sua Vez

A tray icon that lights up when Claude Code finishes, or is waiting on you.<br>
On Windows and macOS.

<sub><i>sua vez</i>: Portuguese for <i>your turn</i></sub>

</div>

You give Claude Code a task, switch to something else, and come back twenty
minutes later. Then you find it stopped after thirty seconds to ask whether it
could run `npm install`. Sua Vez is a speech bubble in the tray that turns amber
when Claude is blocked on you and green when it's done, with a notification for
each.

It's a single 0.5 MB binary with no runtime dependencies. In the tray it uses
about 2 MB of memory and no measurable CPU.

## The bubble

| | |
|---|---|
| **Outline** | Nothing going on. |
| **Outline with dots** | Claude is working. |
| **Green** | Claude finished. Your turn. |
| **Amber with `!`** | Claude is blocked on a permission prompt or a question. |

With several sessions open, the bubble shows the most urgent one. Click it,
with either button, to open the menu. On macOS the quiet states follow the menu
bar's tint, and the two that need you keep their colour.

## The menu

Every open session is listed by project folder, most urgent first:

```
1 needs you · 1 your turn · 1 working
─────────────────────────────────────
⚠  cota wants to use Bash · 2m ago
●  cafe: your turn · 5m ago
…  presence-sync is working · 1m
```

Click a row to mark it as seen. A seen session stops colouring the bubble until
it finishes again.

| | |
|---|---|
| **Mark all as seen** | Clears the colour from every finished or blocked session. |
| **Forget all sessions** | For a session stuck on "working" after a terminal crashed. Live sessions come back with their next event. |
| **Notify when Claude finishes** | A notification when a turn ends. |
| **Notify when Claude needs you** | A notification when Claude is blocked on you. |
| **Edit settings…** | Opens `config.json`. [See below.](#configuration) |
| **Connect to Claude Code** | Adds or removes Sua Vez's hooks in Claude Code's settings. |
| **Run at login** | Per-user, no admin. |

### When it notifies

A notification for every reply would be noise when you're watching the
terminal anyway, so two rules keep it quiet:

- A turn that ends in under 30 seconds doesn't notify. You were almost
  certainly still looking.
- A permission prompt you answer within 8 seconds doesn't notify either.

Both are adjustable. Sessions that finished before Sua Vez started still turn
the bubble green, but they don't send a burst of old notifications.

Running `suavez` again while it's already in the tray makes the running copy
tell you which sessions are waiting.

## Install

Not published to winget, Chocolatey or Homebrew yet. [Build it](#build),
then run it once.

On first launch Sua Vez connects to Claude Code by itself, since that's the
only thing it's for. It adds its hooks to `~/.claude/settings.json`, or to
`$CLAUDE_CONFIG_DIR/settings.json` if you've set that, and tells you it did.
Claude Code reads hooks when a session starts, so sessions that are already
open need a restart before they show up.

To do it from a terminal instead:

```
suavez install      # connect
suavez uninstall    # disconnect
suavez status       # what every session is doing right now
```

The Windows installer removes the hooks when you uninstall. If you delete a
portable copy, run `suavez uninstall` first, or untick **Connect to Claude
Code**.

## How it works

Claude Code [hooks](https://docs.claude.com/en/docs/claude-code/hooks) run a
command on certain events and pass it a JSON description on stdin. Sua Vez
registers itself, `suavez hook`, for six of them:

| Event | Becomes |
|---|---|
| `SessionStart` | ready |
| `UserPromptSubmit` | working, and the turn's clock starts |
| `PostToolUse` | working (any permission prompt has been answered) |
| `Notification` | needs you, for a permission prompt or a question |
| `Stop` | your turn |
| `SessionEnd` | the session is removed |

The hook process writes one small JSON file per session under `sessions/` in
the data folder, pokes the tray, and exits in about 50 ms. It prints nothing
and always exits 0, so a failure in Sua Vez never stops Claude. The tray rereads
the folder when poked, plus once every 15 seconds as a fallback and to drop
sessions that have been quiet for 8 hours. Those are usually ones whose
terminal closed without a `SessionEnd`.

When Sua Vez edits `settings.json`, it only adds or removes entries whose
command is its own `hook`. Your other hooks and settings stay as they were, in
the same order. If `settings.json` isn't valid JSON, it leaves the file alone
and tells you why. If you move the binary, the next launch points the hooks at
the new location.

## Configuration

`%APPDATA%\SuaVez\config.json` on Windows,
`~/Library/Application Support/SuaVez/config.json` on macOS.

```json
{
  "notifyDone": true,
  "notifyNeedsYou": true,
  "minTurnSeconds": 30,
  "graceSeconds": 8,
  "staleHours": 8,
  "hooksOffered": true
}
```

| Key | |
|---|---|
| `minTurnSeconds` | Turns shorter than this don't notify when they end. `0` notifies every time. |
| `graceSeconds` | How long a permission prompt waits before it notifies. |
| `staleHours` | Sessions quiet for this long are dropped. |
| `hooksOffered` | Set after the first launch connects to Claude Code, so disconnecting stays disconnected. |

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

## Not yet

- winget, Chocolatey and Homebrew packages, and a release workflow.
- Clicking a session to bring its terminal to the front.
- Claude Code running inside WSL. Its hooks run on the Linux side and can't
  reach a Windows tray.

## License

MIT
