# Security policy

## Report a vulnerability

Report vulnerabilities privately through GitHub's private vulnerability
reporting: open the repository's **Security** tab and choose **Report a
vulnerability**. **Do not open a public issue, pull request or discussion for a
vulnerability**, and do not include exploit details in one.

Include what you found, the affected version or commit, the steps to reproduce
it and the impact you expect. Use invented data in any example; never send real
recordings, transcripts, tokens or keys. You will get an answer in the private
report, where the fix and its disclosure are coordinated.

## Supported versions

Only the latest release receives security fixes. Orbit is beta software; fixes
are not backported to earlier versions. Upgrade the hub first, then the Mac.

## What Orbit protects

- **Data on the Mac.** Audio waiting for the hub, local voice-note audio and
  pending private notes are encrypted at rest (XChaCha20-Poly1305) with a
  storage key kept in the macOS Keychain. The Mac keeps no copy of the hub's
  conversations, meetings or notes. Logs exclude conversation, transcript,
  note and credential content. Anonymous error reports are off by default and
  carry only codes and numbers.
- **Device credentials.** The device token and the storage key are kept in the
  Keychain, with no plaintext fallback in a normal build. The hub stores only
  a hash of the token, and the hub administrator can revoke a device.
- **Provider credentials stay on the hub.** Model, transcription and Hermes
  credentials live on the server; the Mac never receives them.
- **Transport.** The app talks to the hub only over HTTPS and WSS, except for
  explicit localhost development and the separate test build's private test
  network. The hub-to-translator bridge uses separate
  HMAC keys with route-bound signatures and rejects replayed requests.
- **The privacy stop** cuts capture, playback, transfers and computer control
  on the Mac. It is local and durable: it survives quitting, and no traffic
  from the hub or the assistant can lift it; only the user rearms it on the Mac.

## What it does not protect

- **The hub operator can read your meetings and conversations.** The hub
  encrypts its database, but it holds the key, and it sends audio and
  transcripts to the transcription and note services its operator configures.
  Hermes, and the model providers it uses, see the conversations. There is no
  end-to-end encryption between the Mac and the hub: run your own hub or use
  one whose operator you trust.
- **Computer control is governed by macOS permissions, not by Orbit.** When
  Accessibility and Screen Recording are granted, the assistant can see the
  screen and act on the Mac through the hub, without a per-action approval in
  Orbit. Anyone who can drive your Hermes profile can do the same. Withdraw
  those permissions in System Settings, or use the privacy stop, to prevent it.
- **The local account.** Orbit does not defend against root, or against
  another process running as your user that tampers with its data directory.
- **Builds you did not sign.** An ad hoc development build is not notarized;
  package checksums establish that the bytes match the published manifest, not
  who published them.

The [architecture](docs/architecture/README.md#storage-and-trust) and
[local storage](docs/architecture/storage.md) documents describe each secret,
its holder and its limits in detail.
