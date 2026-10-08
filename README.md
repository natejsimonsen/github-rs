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

## Install

Pick one. Each gives you the same app.

**macOS, with Homebrew** (an app in /Applications, Apple silicon and Intel):

```sh
brew install --cask --no-quarantine natejsimonsen/tap/github-prs
```

`--no-quarantine` matters: the app is signed without an Apple developer
account, so without it macOS blocks the first launch. If you forget, open it
once from System Settings > Privacy & Security > Open Anyway.
Update later with `brew upgrade --cask github-prs`.

**macOS or Linux, with [Flox](https://flox.dev)**:

```sh
flox install github:natejsimonsen/github-rs
```

This builds the app from source the first time (a few minutes) and puts
`github-prs` on your path. Run it with `github-prs`.

**macOS or Linux, with Nix** (flakes enabled):

```sh
nix profile install github:natejsimonsen/github-rs
# or try it without installing:
nix run github:natejsimonsen/github-rs
```

**Download**: every release on the
[Releases page](https://github.com/natejsimonsen/github-rs/releases) has a
zipped `GitHub PRs.app` for macOS and a Linux x86_64 binary, with checksums.

**From source, with Cargo** (any OS):

```sh
cargo install --git https://github.com/natejsimonsen/github-rs
```

Linux needs a C compiler and the window-system libraries at run time
(`libxkbcommon`, `wayland` or X11, `libGL` or Vulkan), e.g. on Ubuntu:
`sudo apt install build-essential libxkbcommon0 libwayland-client0 libgl1 libvulkan1`.
Windows needs the Visual Studio C++ build tools (rustup offers to install them).
Windows and Linux builds compile in CI but haven't been used day to day yet.

## Develop

Clone the repo, then get a toolchain one of these ways:

- **Flox**: `flox activate` gives you cargo, rustc, clippy, rustfmt and
  rust-analyzer, plus the Linux libraries. Leave with `exit`.
- **Nix**: `nix develop` does the same.
- **rustup**: install from https://rustup.rs. If `cargo` isn't found afterward,
  add it to your shell: `echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc`
  and open a new terminal.

Then:

```sh
cargo run --release
```

The first build takes about 2 minutes. Later builds take seconds. The
finished program is `target/release/github-prs`. Plain `cargo run` also works
and is still fast, because the project optimizes its libraries even in debug
builds.

**macOS app bundle**: `scripts/bundle-macos.sh` builds `GitHub PRs.app` (with
its icon) into `/Applications`, so Spotlight and the Dock can open it. Run it
again after you change the code. To install somewhere else, pass a folder:
`scripts/bundle-macos.sh ~/Applications`.

**Checks before you push**: CI runs `cargo fmt --check`,
`cargo clippy --all-targets --features snapshot -- -D warnings`, `cargo test`,
a release build on macOS, Linux and Windows, and `nix build` on macOS and
Linux.

**Releasing** is automatic. Write commit messages as
[Conventional Commits](https://www.conventionalcommits.org): `fix: ...` makes a
patch release, `feat: ...` a minor one, and `feat!: ...` (or a
`BREAKING CHANGE:` footer) a major one. Other types (`docs:`, `chore:`, ...)
don't release. Once CI passes on `main`, the Release workflow bumps
`Cargo.toml`, commits and tags `vX.Y.Z`, builds a universal macOS app and a
Linux binary, and publishes them on the Releases page. The Homebrew tap
([natejsimonsen/homebrew-tap](https://github.com/natejsimonsen/homebrew-tap))
checks for new releases every few hours and updates its cask; run its
"Update cask" workflow to do it right away. Flox and Nix build from the
latest commit on `main`; `flox upgrade` picks up a new release.

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
| `flake.nix`, `nix/` | Nix package and dev shell (Flox installs from these) |
| `.flox/` | Flox dev environment |
| `.github/workflows/` | CI and release pipelines |

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
