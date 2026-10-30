# Security

## Reporting

Use GitHub's private vulnerability reporting on this repository (Security tab, "Report a
vulnerability"). Do not open a public issue for a security problem. Do not include captured
playlists, tokens or your network's addresses in a report.

## What to expect from the service

- **It has no authentication.** Anyone who can reach its port can watch, read chat and change the
  followed-channel list. Run it on your LAN, behind your VPN, or behind a reverse proxy that signs
  people in. Never publish its port to the Internet as it is.
- **It makes outbound requests on behalf of callers**, but only to fixed hosts: Twitch's GraphQL and
  playlist servers, Twitch's chat server, and the three emote services. Channel names are validated
  (`[A-Za-z0-9_]{2,25}`) before they go into any address.
- **It stores two files**: the followed channels and a log of advert breaks met. Nothing about who
  watched.
- **The container** runs read-only, as a non-root user, with all capabilities dropped and a `/tmp`
  tmpfs. Keep it that way in your own compose file.
- **The VPN exit** (gluetun) needs `NET_ADMIN` and `/dev/net/tun`. Breakless only needs the exit's
  HTTP proxy on a Docker network; do not publish the proxy's port.

Nothing here has had an independent security review.
