# Contributing to Otomo

Thanks for helping. Bug reports, ideas, documentation and code are all welcome, in English or Japanese. 日本語でも大丈夫です。

## Before you start

- **Bugs**: open an [issue](https://github.com/hibiki1213/otomo/issues/new/choose).
- **Questions and ideas**: start a [discussion](https://github.com/hibiki1213/otomo/discussions).
- **Large changes**: talk it through in an issue or discussion before writing code, so we agree on the direction first. Small fixes can go straight to a pull request.
- **Security issues**: report them privately, as described in [SECURITY.md](SECURITY.md).

## Setting up

Follow [Build from source](README.md#build-from-source) in the README. In short, with the Xcode Command Line Tools, Rust, Node.js with pnpm, CMake, and cargo-about installed:

```sh
pnpm install
pnpm mcp:build   # the MCP server Claude connects to; the app expects it beside itself
pnpm app:dev     # runs the app with hot reload (and fetches the bundled SoundFont the first time)
```

The README's "Where things are" table shows how the code is laid out.

## Checking your change

```sh
pnpm build                              # type-checks and builds the UI
pnpm sounds                             # the bundled SoundFont, if app:dev hasn't fetched it yet
pnpm licenses                           # the license notices the app ships; fails on a dependency under an unlisted license
cd src-tauri && cargo test --workspace  # the app and the MCP server
```

CI runs the same on every pull request. Try your change in the running app too: most of Otomo is something you see or hear.

## Writing code here

- **Match what is around it.** Naming, comments and structure should read like the surrounding code. Comments explain why, not what.
- **Every UI string in both languages.** Add it to `src/locales/en.ts` and `src/locales/ja.ts`, or use `tr!("日本語", "English")` in Rust. If you can't write one of them, say so in the pull request and we'll fill it in.
- **No new network dependency at runtime** without talking about it first. Otomo should keep working offline once its sounds are downloaded.
- **Claude's edits stay pending.** Anything Claude can change through the MCP tools must go through the pending-edits queue, never straight into the song.

## Music files

Songs in tests, examples, issues and screenshots must be **your own or in the public domain**. Please don't add MIDI or scores of copyrighted music, even as a test fixture.

## Using AI to write code

You're welcome to use AI tools; Otomo itself is built with Claude. What you submit is still yours to stand behind: understand every line, and run it yourself before opening the pull request.

## Licensing

Otomo is released under the [MIT License](LICENSE). By contributing, you agree that your contribution is licensed under it too. There is no CLA to sign.

The name and icon are not covered by the license; see [TRADEMARKS.md](TRADEMARKS.md).

## Code of conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
