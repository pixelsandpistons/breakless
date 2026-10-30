# Third-party notices

## Bundled

| What | Where | Licence |
|---|---|---|
| [hls.js](https://github.com/video-dev/hls.js) 1.5.17, minified | `web/hls.min.js` | Apache-2.0. Copyright Dailymotion and contributors; parts derived from videojs-contrib-hls, copyright Brightcove. The licence text is in the project's repository and applies to this file |

## Used, not bundled

| What | How | Licence |
|---|---|---|
| [gluetun](https://github.com/qdm12/gluetun) | the VPN exit container in `deploy/docker-compose.yml` | MIT |
| Twitch | the service talks to Twitch's unofficial GraphQL and playlist endpoints; see the README's notice | Twitch's terms apply to you |

## Knowledge, not code

The list of marks that identify an advert segment was assembled by reading real playlists and
checking them against what two other projects look for: the `vaft` userscript in
[TwitchAdSolutions](https://github.com/ryanbr/TwitchAdSolutions) (MIT) and
[Streamlink](https://github.com/streamlink/streamlink)'s Twitch plugin (BSD-2-Clause). No code was
copied from either. The upstream watcher (planned) downloads their current source at run time into
an ignored folder to compare; it is not redistributed here.
