# GitHub PRs

A fast desktop app for GitHub pull requests, styled like github.com. Written in Rust.

It shows your PRs, their conversation, commits, CI checks, and diffs. You can
comment, approve, or request changes. It opens to **PRs you created** (10 at a
time); the other tabs show review requests, assigned, mentioned, and involved.
The search box takes normal GitHub search syntax (`repo:org/name label:bug`).
Press **⌘K** to jump to any PR: type a number (`119316`), `owner/repo#123`,
or paste a link. A pasted link opens right away.

The timeline shows what happened, like on GitHub: commits pushed, labels,
force-pushes, review requests, merges, and closes. Clicking a commit opens
its own page in the app, with its message, parents, and diff; Esc goes back.
Files changed has a folder tree with a filter box, and each wide file scrolls
sideways on its own.
Checks lists failures first, marks required checks, and folds skipped ones.

In the conversation you can reply to code review threads, **Resolve** or
**Unresolve** them, and **Quote reply** any comment from its ⋯ menu. The
comment box has **Write / Preview** tabs; ⌘Enter posts. Comments render
GitHub-style: tables, task lists, `> [!NOTE]` alerts, @mentions, #123 links,
bare URLs, suggested changes, and color emoji (both 🎉 and `:tada:`), drawn
with the same emoji pictures github.com uses.

Code is syntax highlighted with tree-sitter in GitHub's colors: in diffs,
review thread snippets, and fenced code blocks. About 110 languages are
built in, picked by file name or the code block's language tag.

Each PR shows its merge status at a glance: ready, blocked (and why),
conflicts, out of date, or auto-merge on. It appears in the list rows and in a
colored bar under the PR title. The bar also has a **Merge** button (with a
merge-method menu and a confirm step) and **Enable / Disable auto-merge**.

The app checks merge requirements itself rather than trusting GitHub's summary
alone: an approval from someone else, required checks passing, all
conversations resolved, and no conflicts. **policy-bot is always shown as
passing**, since it reports the same failure on every PR.

In **Files changed**, each file has a **Viewed** checkbox, just like on
github.com. It syncs with GitHub, folds the file away, and the top shows
"N / M files viewed".

Every sidebar collapses: the PR list (to a strip of PR icons), the details
column next to the conversation, and the file list next to the diff. Click the
panel icon, use the shortcut, or drag a panel's edge closed. The app remembers
what you left open.

## Build and run

1. Install Rust from https://rustup.rs (one command; it installs `cargo`).
   If `cargo` isn't found afterward, add it to your shell path:
   `echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc` and open a new terminal.
2. Build and start the app:

   ```sh
   cd ~/Code/github-rs
   cargo run --release
   ```

   The first build takes about 2 minutes. Later builds take seconds.
   The finished program is `target/release/github-prs`. You can copy it anywhere
   and run it directly.

3. **Optional, macOS:** install it as a normal app, so you can open it from
   Spotlight, Launchpad, the Dock, or any app launcher:

   ```sh
   scripts/bundle-macos.sh
   ```

   This builds `GitHub PRs.app` (with its icon) into `/Applications`. Run it
   again after you change the code. To install somewhere else, pass a folder:
   `scripts/bundle-macos.sh ~/Applications`.

Use `--release` for everyday use. Plain `cargo run` also works and is still
fast, because the project optimizes its libraries even in debug builds.

**Linux** also needs a C compiler and window-system headers, e.g. on Ubuntu:
`sudo apt install build-essential libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
**Windows** needs the Visual Studio C++ build tools (rustup offers to install them).
Windows and Linux builds have not been tested yet.

## Signing in

The app finds a token in this order:

1. `GH_TOKEN` or `GITHUB_TOKEN` environment variable.
2. A token it saved earlier (see below).
3. Your `gh` CLI login. The app reads the credential `gh` stored in the OS
   keychain directly, without running `gh`. macOS asks once whether the app
   may read it.
4. Otherwise it asks you to paste a personal access token (scopes: `repo`, `read:org`).

After it finds a token in steps 3–4, the app saves it to a file that only
your user can read, so the keychain prompt doesn't come back:
`~/Library/Application Support/github-prs/token` on macOS
(`~/.config/github-prs/token` on Linux, `%APPDATA%\github-prs\token` on Windows).
To sign out, right-click your avatar (top right), or delete that file.

## Keyboard

| Keys | Action |
| --- | --- |
| `↑` `↓` or `j` `k` | Previous / next PR |
| `⌘1` – `⌘4` | Conversation, Commits, Checks, Files changed |
| `⌘B` | Show or hide the PR list |
| `⇧⌘B` | Show or hide the details column / file list |
| `⌘R` | Refresh |
| `⌘K` | Go to a PR by number or link |
| `/` or `⌘F` | Search |
| `⌘O` | Open PR on github.com |
| `c` or `⇧⌘C` | Copy PR link (`c` works when you're not typing) |
| `⌘W` or `Ctrl+W` | Close the window |

On Windows and Linux, use `Ctrl` instead of `⌘`. Double-click a PR to open it on github.com.

## How it works

- **UI: [egui](https://github.com/emilk/egui).** A Rust library that draws the
  whole interface on the GPU (Metal on macOS), like a game does. No browser or
  web view is involved. Drawing everything itself lets the app copy GitHub's
  look exactly and run on macOS, Windows, and Linux from one codebase. It only
  redraws when something changes, and a frame takes about 1–2 ms.
- **Data: GitHub's JSON APIs.** GraphQL loads a list or a whole PR page in one
  request. REST is used for file diffs and for posting comments and reviews.
  Network calls run on background threads, so the window never freezes.
- **Speed tricks.** GitHub's search takes 1–3 seconds, so the app works around it:
  - Lists load in two steps: a lean search shows the rows (~1 s), then one
    request by PR id adds labels, avatars, and CI status.
  - Merge status is slow for GitHub to compute (1–3 s), so it has its own
    request and never holds up the list. If GitHub is still computing it,
    the app asks again a few seconds later.
  - The top 10 PRs are fetched in the background, so clicking one is instant.
  - Everything is cached on disk (`~/Library/Caches/github-prs`). Later
    launches show the last data in under 0.1 s, then refresh quietly.
- **Only native APIs.** Credentials use the OS keychain API (`keyring`).
  Links open with `NSWorkspace` (macOS), `ShellExecuteW` (Windows), or the
  desktop portal over D-Bus (Linux). HTTPS certificates are checked against
  the OS trust store. The app never starts another program.

## Code map

| File | What it does |
| --- | --- |
| `src/main.rs` | Opens the window |
| `src/app.rs` | App state, background loading, keyboard shortcuts |
| `src/github.rs` | GitHub API calls and data types |
| `src/auth.rs` | Finds and saves the token |
| `src/cache.rs` | Disk cache |
| `src/theme.rs` | GitHub's colors (light and dark) and system fonts |
| `src/icons.rs` | Octicon-style icons, drawn as shapes |
| `src/views/` | The list and PR page |
| `src/diff.rs` | The Files changed view (draws only visible lines) |
| `src/images.rs` | Downloads avatars, images in comments, and emoji |
| `src/gfm.rs` | GitHub's Markdown extras: mentions, #refs, emoji |
| `src/syntax.rs` | Syntax highlighting (tree-sitter grammars via arborium) |
| `src/snapshot.rs` | Offscreen screenshots for UI reviews (optional feature) |
| `scripts/bundle-macos.sh` | Builds and installs the macOS `.app` |
| `examples/make_icon.rs` | Draws the app icon |

## Developer switches

- `GITHUB_PRS_PERF=1` prints how long each frame and network reply takes.
- `GITHUB_PRS_SCREENSHOT=out.png` saves a picture of the window once data
  loads, then quits. Combine with `GITHUB_PRS_TAB=files|commits|checks`,
  `GITHUB_PRS_THEME=dark|light`, `GITHUB_PRS_ROW=<n>`, and
  `GITHUB_PRS_COLLAPSE=list,details,tree`, `GITHUB_PRS_SCROLL=<px>|bottom`,
  `GITHUB_PRS_GOTO=<link>`, `GITHUB_PRS_COMMIT=first|last|<sha>` (open that
  commit's page), and `GITHUB_PRS_STATE=reply,quote,preview`.
  These runs don't save settings.
- Offscreen screenshots (work even with the screen locked, and block every
  write to GitHub): build with `cargo build --release --features snapshot`,
  then add `GITHUB_PRS_OFFSCREEN=1`. `GITHUB_PRS_SCRIPT` can click, type,
  and press keys first; see `src/snapshot.rs`.
