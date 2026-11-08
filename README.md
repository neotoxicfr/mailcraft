# MailCraft

An open-source desktop mail client for Windows, in pure Rust, in the spirit of the
[ArtCraft Crafting Apps](https://github.com/storytold): the familiar look of Outlook (classic
ribbon, three panes), built only from free resources.

Beyond reading and writing mail, MailCraft helps clean up a mailbox: it groups mail by sender,
reads the `List-Unsubscribe` and authentication headers (SPF, DKIM, DMARC), flags lookalike and
phishing senders, unsubscribes in one click where the sender supports it (RFC 8058), and can ask
Claude to classify senders when an API key is set (optional).

**Status: early work.** Milestone 1 is the shell only: window, ribbon, panes, light/dark theme,
accent colour (Outlook blue, Windows accent or MailCraft teal), French and English. No account can
be added yet.

## Planned

- Accounts: Microsoft (Outlook.com/Hotmail, through Microsoft Graph), Gmail, any IMAP/SMTP server.
- Read, write, folders, local full-text search; simplified HTML rendering by default, the original
  on request.
- Clean-up view per sender, safe unsubscribe, bulk archive/delete, block rules, action log, undo.
- Dry-run by default, allow-list of senders never touched, no automatic unsubscribe for suspect
  senders.
- Optional: tracker blocking, screening of new senders, newsletter digest.
- Windows installer (MSI), start at sign-in (minimised or not), notification-area icon.

## Build

Rust 1.95 or later, Windows with the MSVC toolchain.

```
cargo run -p mailcraft
cargo test --workspace
```

Settings live in `config.toml` in the per-user configuration folder; set `MAILCRAFT_CONFIG_DIR`
to use another folder.

## Licence

MIT or Apache-2.0, at your option. Third-party material (fonts) is listed in
[ATTRIBUTION.md](ATTRIBUTION.md) and [NOTICE](NOTICE). MailCraft is not affiliated with
Microsoft; "Outlook" is a trademark of Microsoft and is only used to describe the look.
