# Task 04: a fake Twitch for the service's tests

Status: open.

## Goal

A test-only crate, `crates/fake-twitch`, that stands in for Twitch's three endpoints on loopback so
the service's session and fallback logic can be tested without the network. It is the test the
prototype never had.

## Read first

- `reference/python/twitchlib.py`: `TOKEN_QUERY`, `master_url` (the query string the service sends),
  `master_info` and `variants` (what a master playlist must contain for the service to accept it),
  `live` and `top` (the two other GraphQL queries and their response shapes).
- `fixtures/playlists/`: real master and media playlists to serve.
- `reference/python/server.py`, `Channel.open` and `Channel.media`: what the service does on 404,
  on a dropped connection, on 5xx, and how it decides a channel is offline.

## Interface

```rust
pub struct FakeTwitch { pub gql: SocketAddr, pub usher: SocketAddr, pub cdn: SocketAddr, .. }
impl FakeTwitch {
    pub async fn start() -> Self;
    pub fn channel(&self, login: &str) -> ChannelHandle;        // creates a live channel
}
impl ChannelHandle {
    pub fn offline(&self);                                       // usher answers 404 from now on
    pub fn advert_break(&self, segments: usize, roll: &str);     // the next media playlists are all adverts for N segments, then stream resumes with a discontinuity
    pub fn drop_next(&self, n: usize);                           // the next N requests close the connection without a response
    pub fn status(&self, code: u16, n: usize);                   // the next N requests answer this status
    pub fn country(&self, cc: &str);                             // what USER-COUNTRY says in the master
    pub fn requests(&self) -> Vec<Request>;                      // what was asked, in order, with the Via header the proxy adds
}
```

The media playlist must advance as Twitch's does: a sliding window of 2-second segments, new segment
every 2 s (configurable faster for tests), `#EXT-X-MEDIA-SEQUENCE` increasing, `#EXT-X-PROGRAM-DATE-TIME`
on each segment, advert segments titled `Amazon|<id>` with a `twitch-stitched-ad` `DATERANGE` whose
`START-DATE` and `DURATION` cover them.

Also provide an **HTTP proxy** that the service can be pointed at (`CONNECT` is not needed; the
service's `reqwest` will send absolute-form requests for `http://` and `CONNECT` for `https://`: serve
plain `http://` from the fake, so the proxy only needs absolute-form forwarding) with
`proxy.down()` and `proxy.up()` to simulate the exit failing.

## Dependencies

`axum`, `tokio`, `hyper` as the service already uses; nothing new if possible.

## Accepted when

- The crate's own tests show: a channel serves a master whose `variants` parse; the window advances;
  `advert_break` produces playlists that `breakless_formats::parse_media` marks as adverts and then
  a clean playlist with a discontinuity; `drop_next` and `status` behave; the proxy forwards and can
  go down.
- It is `publish = false` and only a `dev-dependency` of the service.
