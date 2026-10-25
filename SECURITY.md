# Security

## Reporting a vulnerability

Please report security issues privately, through GitHub's [private vulnerability reporting](https://github.com/hibiki1213/otomo/security/advisories/new). Don't open a public issue for them. English or Japanese is fine.

You can expect a first reply within a week. Once a fix is released, the advisory is published, with credit to you if you'd like it.

## Supported versions

Only the latest release gets security fixes.

## What is in scope

Otomo runs on your Mac and has no server of its own. The parts most worth a look:

- **The MCP server** (`otomo-mcp`), which Claude Desktop and the in-app chat start, and the local control socket the app listens on.
- **Claude Desktop's settings file**, which Otomo edits when you press "Connect to Claude".
- **The in-app chat**, which runs `claude` with a restricted set of tools.
- **File parsing**: MIDI, MusicXML (including `.mxl`), SFZ and SoundFont files.
- **Sample library downloads**: fetching the archives and unpacking them into the app's data folder.

Vulnerabilities in Claude Code, Claude Desktop or the Claude models themselves should go to [Anthropic](https://www.anthropic.com/responsible-disclosure-policy).
