# MailCraft

An open-source desktop mail client for Windows, in pure Rust, in the spirit of the
[ArtCraft Crafting Apps](https://github.com/storytold): the familiar look of Outlook (classic
ribbon, three panes), built only from free resources.

Beyond reading and writing mail, MailCraft helps clean up a mailbox: it groups mail by sender,
reads the `List-Unsubscribe` and authentication headers (SPF, DKIM, DMARC), flags lookalike and
phishing senders, unsubscribes in one click where the sender supports it (RFC 8058), and can ask
Claude to classify senders when an API key is set (optional).

**Status: early work.**

- Milestone 1: window, ribbon, panes, light/dark theme, accent colour (Outlook blue, Windows
  accent or MailCraft teal), French and English.
- Milestone 2: Microsoft accounts (Outlook.com/Hotmail and work accounts). Sign-in in the browser
  (OAuth 2.0 + PKCE, nothing typed into MailCraft), sync of the last 6 months then changes only,
  folder tree, message list grouped by date with search, reading pane (plain text for now).
  The local copy is encrypted by default (SQLCipher); the key lives in Windows Credential Manager
  and also seals the sign-in tokens, so no secret is ever stored in clear.
- Milestone 3: a complete client. Delete, move, drag and drop, flags, undo (Ctrl+Z), folders;
  offline full-text search plus server search; formatted reading (pure-Rust simplified view, or
  the original through Edge WebView2 with scripts off); attachments; writing with a rich-text
  editor, replies, forwards, drafts, large attachments, signature, address completion.
- Milestone 5: clean-up view. Per sender: mail count, unread share, SPF/DKIM/DMARC, how to
  unsubscribe (RFC 8058 one-click, e-mail or web page), and warnings (brand impersonation,
  lookalike, punycode, generated or impossible domains, mail "from yourself"). Grouped
  unsubscribe, delete all, archive all, block (junk + rule), allow-list, action log. Simulation is
  on by default, suspect senders are never unsubscribed from automatically.
- Milestone 7: File › Options, a skippable first-launch assistant, screening of new senders,
  tracker blocking.
- Milestone 8: notification-area icon, start with Windows (minimised or not), close to the
  notification area, one running instance, new-mail notifications, MSI installer (built by the
  Release workflow).

- Milestone 6: with an Anthropic API key (optional, kept in Windows Credential Manager), Claude
  sorts the senders of the clean-up view and summarises the last day of newsletters.
- Milestone 4: other accounts over IMAP + SMTP with an app password: Gmail, Yahoo, iCloud,
  Orange, Free, La Poste, SFR, GMX, AOL or any server (filled in from the address). Same
  features as Microsoft accounts; blocking a sender files its new mail in Junk at each sync.

## Planned

- Gmail sign-in in the browser (OAuth) instead of an app password.
- Calendar and contacts.

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
