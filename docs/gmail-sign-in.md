# Google accounts (Gmail, contacts, calendar) through the browser

Gmail mail works out of the box with an **app password** (File › Add an account › Other
account). The browser sign-in (the "Google account (Gmail)" button) also gives access to
Google contacts and calendar, and needs a Google Cloud project; its identifiers are given to
the build, never written in the source.

## 1. Create the project (once, about 15 minutes, free)

1. <https://console.cloud.google.com/> → project selector → **New project** → name it
   `MailCraft` → Create, then select it.
2. **APIs & Services › Library**: enable **Gmail API**, **Google People API** and
   **Google Calendar API**.
3. **Google Auth Platform** (formerly "OAuth consent screen") → Get started:
   - **Branding**: app name `MailCraft`, user support e-mail, developer contact e-mail.
     Only the test users below ever see them.
   - **Audience**: **External**, keep the status **Testing**, and add each Google address
     that will use MailCraft as a **test user** (up to 100).
   - **Data access** › Add or remove scopes: `https://mail.google.com/`,
     `.../auth/contacts`, `.../auth/contacts.other.readonly`, `.../auth/calendar`, plus
     `openid`, `.../auth/userinfo.email`, `.../auth/userinfo.profile`. Save.
4. **Clients** › **Create client** → application type **Desktop app** → name
   `MailCraft desktop` → Create → **Download JSON**.

Testing mode needs no review by Google. Its limit: Google asks the test users to sign in
again every 7 days. Publishing the app removes that limit but asks Google to verify it
(restricted Gmail scope): not needed for personal use.

## 2. Give the identifiers to the build

- **Local build**: save the downloaded file as `google-client.json` at the root of the
  repository (next to `Cargo.toml`). It is ignored by git (`.gitignore`), and read at build
  time by `crates/sync/build.rs`.
- **Release builds (GitHub Actions)**: repository › Settings › Secrets and variables ›
  Actions › New repository secret: `MAILCRAFT_GOOGLE_CLIENT_ID` and
  `MAILCRAFT_GOOGLE_CLIENT_SECRET` (the `client_id` and `client_secret` of the file). The
  Release workflow passes them to the build. With the GitHub CLI, each command asks for the
  value without showing it:

  ```
  gh secret set MAILCRAFT_GOOGLE_CLIENT_ID --repo <owner>/mailcraft
  gh secret set MAILCRAFT_GOOGLE_CLIENT_SECRET --repo <owner>/mailcraft
  ```

Without them the Google button is hidden; nothing else changes. For a desktop app Google
says the client secret is not confidential, but it stays out of the repository anyway
(secret scanners flag it).

## How it works

Authorization code with PKCE and a loopback redirect (`http://localhost:<port>`), like the
Microsoft sign-in; `access_type=offline` gives a refresh token, kept sealed in the local
store with the vault key. IMAP and SMTP authenticate with XOAUTH2 and a short-lived access
token renewed a minute before it expires. Address and name come from the OpenID `userinfo`
endpoint. All scopes are asked at the first sign-in, so contacts and calendar will need no
new consent.
