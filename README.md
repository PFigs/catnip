# Catnip

A desktop app to review GitHub pull requests from the keyboard. It is written in Rust with
[Rune](https://github.com/anomalyco/rune).

![Catnip](assets/screenshot.png)

- It lists the open PRs of the orgs, users or repositories you pick, with their CI state and
  your review state.
- It shows the conversation, the diff and the review threads of a PR in one window.
- You can approve, comment, reply to and resolve threads, and run CI again.
- Press `?` to see the keys. A cat lives in it.

## Try it

Catnip signs in through the [GitHub CLI](https://cli.github.com): run `gh auth login` once.
It also takes a classic token with `repo` and `read:org` from `GITHUB_TOKEN`.

macOS, Apple Silicon:

```sh
curl -fsSL https://github.com/PFigs/catnip/releases/latest/download/Catnip-macos-arm64.zip -o /tmp/Catnip.zip && ditto -x -k /tmp/Catnip.zip /Applications && open /Applications/Catnip.app
```

The app is not signed. If you download the zip in a browser instead, run
`xattr -dr com.apple.quarantine /Applications/Catnip.app` before you open it.

Linux, x86_64:

```sh
curl -fsSL https://github.com/PFigs/catnip/releases/latest/download/catnip-linux-x86_64.tar.gz | tar -xz -C ~/.local/share && ~/.local/share/catnip/catnip
```

## Build from source

You need [Rust](https://rustup.rs). On macOS you also need the Xcode command line tools
(`xcode-select --install`). On Debian or Ubuntu you also need
`sudo apt-get install libxkbcommon-dev libwayland-dev`.

```sh
git clone https://github.com/PFigs/catnip && cd catnip && cargo run --release
```

On Linux, `mise run desktop` installs a launcher and the icon.

## Release

Push a tag that matches the version in `Cargo.toml`, such as `v0.1.0`. The `Release` workflow
builds both platforms and publishes them as a GitHub Release.

## License

MIT
