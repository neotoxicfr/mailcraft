# Gmail sign-in through the browser

Gmail works out of the box with an **app password** (File › Add an account › Other account).
The browser sign-in (the "Google account (Gmail)" button) needs a Google Cloud project; its
identifiers are given to the build, never written in the source.

## 1. Create the project (once, about 15 minutes, free)

1. <https://console.cloud.google.com/> → project selector → **New project** → name it
   `MailCraft` → Create.
2. **APIs & Services › OAuth consent screen** (Google Auth Platform):
   - User type **External**, app name `MailCraft`, support e-mail: a project address (a
     Google group or alias works; it is shown to people who sign in).
   - **Audience**: keep the app in **Testing** and add each Gmail address that will use
     MailCraft as a **test user** (up to 100). Testing mode needs no Google review.
   - **Data access**: add the scopes `https://mail.google.com/`, `openid`, `email`,
     `profile`.
3. **Clients › Create client** → application type **Desktop app** → name `MailCraft
   desktop` → Create. Note the **client ID** and the **client secret**. For a desktop app the
   secret is not confidential (Google says so), but keep it out of the repository anyway:
   secret scanners flag it.

In Testing mode Google asks test users to sign in again every 7 days. Publishing the app
removes that limit but requires Google's verification for the `mail.google.com` scope
(a security assessment): not needed for personal use.

## 2. Give the identifiers to the build

- **Release builds (GitHub Actions)**: repository › Settings › Secrets and variables ›
  Actions › New repository secret: `MAILCRAFT_GOOGLE_CLIENT_ID` and
  `MAILCRAFT_GOOGLE_CLIENT_SECRET`. The Release workflow passes them to the build.
- **Local build**: set both environment variables before `cargo build`, for example in
  PowerShell:

  ```powershell
  $env:MAILCRAFT_GOOGLE_CLIENT_ID = "…apps.googleusercontent.com"
  $env:MAILCRAFT_GOOGLE_CLIENT_SECRET = "…"
  cargo build -p mailcraft
  ```

Without them the Google button is hidden; nothing else changes.

## How it works

Authorization code with PKCE and a loopback redirect (`http://localhost:<port>`), like the
Microsoft sign-in; `access_type=offline` gives a refresh token, kept sealed in the local
store with the vault key. IMAP and SMTP then authenticate with XOAUTH2 and a short-lived
access token renewed a minute before it expires. Address and name come from the OpenID
`userinfo` endpoint.
