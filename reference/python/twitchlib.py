"""Talking to Twitch without an account: who is live, a playback token, the playlists, and
telling advert segments from stream segments. Standard library only.

Used by server.py (the service), probe.py (measures where adverts are served today) and
test_twitchlib.py. Everything Twitch-specific that can go stale lives in this one file; README.md
section 7 says what to change when Twitch moves something.
"""
import collections
import datetime
import json
import random
import re
import string
import urllib.error
import urllib.parse
import urllib.request

# The public web player's client id. Every third-party player uses it; it is not a secret.
CLIENT_ID = "kimne78kx3ncx6brgo4mv6wki5h1ko"
GQL = "https://gql.twitch.tv/gql"
USHER = "https://usher.ttvnw.net/api/channel/hls/%s.m3u8"
UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
      "Chrome/140.0.0.0 Safari/537.36")

# Token types, as (playerType, platform). probe.py asks for each of them.
PLAYER_TYPES = [
    ("site", "web"),
    ("popout", "web"),
    ("embed", "web"),
    ("mobile_web", "web"),
    ("autoplay", "android"),
    ("picture-by-picture", "web"),
    ("thunderdome", "web"),
    ("frontpage", "web"),
]

TOKEN_QUERY = """query PlaybackAccessToken($login: String!, $playerType: String!, $platform: String!) {
  streamPlaybackAccessToken(channelName: $login, params: {platform: $platform, playerBackend: "mediaplayer", playerType: $playerType}) {
    value
    signature
  }
}"""

# What marks an advert in a media playlist (seen live 2026-10-04, fixtures/playlists/). The same
# evidence vaft and Streamlink use (the vaft userscript line 64,
# the Streamlink Twitch plugin lines 190-198).
#   - a DATERANGE of class twitch-stitched-ad, with a start and a duration
#   - the segment's title: stream segments are titled "live", adverts "Amazon|id" or "DCM|id"
# NOT an advert in the video: a DATERANGE of class twitch-maf-ad. It appears about two and a half
# minutes into every session, from any country, and asks the official player to show an advert of
# its own over the stream ("is_stitched": false inside it). The segments under it are the stream.
# Cutting on it threw away a minute of real stream per session until this was found
# (fixtures/playlists/home-clientside-media.m3u8). We simply do not do what it asks.
# Widened 2026-10-10 from what vaft 68.5.7 looks for (the vaft userscript lines 59-69): the whole
# twitch-stitched-* family of classes (-ad, -mid, -pod), not the -ad one alone; segments whose address
# says advert whatever their title (AD_URLS); and everything between #EXT-X-CUE-OUT and #EXT-X-CUE-IN.
# None of the three was seen here; they cost nothing while Twitch does not use them.
AD_DATERANGE = re.compile(r'CLASS="twitch-stitched|ID="stitched-ad-')
AD_URLS = ("/adsquared/", "/_404/")
CLIENT_SIDE = 'CLASS="twitch-maf-ad"'
STREAM_TITLES = ("live", "")
# DATERANGE classes met so far that say nothing about adverts in the video. A class in neither list is
# reported (parse_media's odd_classes, /api/status) so that somebody looks before it costs a stream.
KNOWN_CLASSES = ("timestamp", "twitch-session", "twitch-stream-source", "twitch-trigger", "twitch-ad-quartile",
                 "twitch-assignment", "twitch-maf-ad")


class TwitchError(Exception):
    pass


def device_id():
    return "".join(random.choices(string.ascii_lowercase + string.digits, k=32))


_direct = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def opener(proxy=None):
    """An opener that goes straight out (proxy=None) or through an HTTP proxy URL."""
    if not proxy:
        return _direct
    return urllib.request.build_opener(urllib.request.ProxyHandler({"http": proxy, "https": proxy}))


def http(url, data=None, headers=None, timeout=10, via=None):
    h = {"User-Agent": UA}
    h.update(headers or {})
    req = urllib.request.Request(url, data=data, headers=h)
    host = urllib.parse.urlsplit(url).netloc
    try:
        with (via or _direct).open(req, timeout=timeout) as r:
            return r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")[:200]
        err = TwitchError("HTTP %s from %s: %s" % (e.code, host, body))
        err.code = e.code
        raise err
    except (urllib.error.URLError, OSError) as e:
        err = TwitchError("%s: %s" % (host, e))
        err.code = 0
        raise err


def gql(query, variables=None, dev=None, via=None):
    body = json.dumps({"query": query, "variables": variables or {}}).encode()
    out = json.loads(http(GQL, body, {"Client-ID": CLIENT_ID, "X-Device-Id": dev or device_id(),
                                      "Content-Type": "text/plain;charset=UTF-8"}, via=via))
    if out.get("errors"):
        raise TwitchError("GQL: " + json.dumps(out["errors"])[:300])
    return out.get("data") or {}


def valid_login(name):
    return bool(re.fullmatch(r"[A-Za-z0-9_]{2,25}", name or ""))


def token(login, player_type="site", platform="web", dev=None, via=None):
    d = gql(TOKEN_QUERY, {"login": login, "playerType": player_type, "platform": platform}, dev, via)
    t = d.get("streamPlaybackAccessToken")
    if not t:
        raise TwitchError("no token for %s as %s" % (login, player_type))
    return t


def master_url(login, tok):
    q = {
        "player": "twitchweb", "p": random.randint(1000000, 9999999), "type": "any",
        "allow_source": "true", "allow_audio_only": "true", "allow_spectre": "false",
        "playlist_include_framerate": "true", "supported_codecs": "h264",
        "fast_bread": "true", "sig": tok["signature"], "token": tok["value"],
    }
    return USHER % login.lower() + "?" + urllib.parse.urlencode(q)


def master(login, player_type="site", platform="web", dev=None, via=None):
    """The master playlist text for a live channel. TwitchError (code 404) when offline."""
    return http(master_url(login, token(login, player_type, platform, dev, via)), via=via)


def master_info(master_text):
    """The facts Twitch states about the viewer in a master playlist: country, cluster."""
    line = next((l for l in master_text.splitlines() if l.startswith("#EXT-X-TWITCH-INFO:")), "")
    get = lambda k: (re.search(r'(?:^|,|:)%s="([^"]*)"' % k, line) or [None, ""])[1]
    return {"country": get("USER-COUNTRY"), "cluster": get("CLUSTER"), "node": get("NODE"),
            "session": get("VIDEO-SESSION-ID")}


def variants(master_text):
    """[{slug, name, bandwidth, resolution, fps, codecs, url}], best first.

    slug is Twitch's own group id ("chunked" is the source quality, then "720p60", "480p30",
    "audio_only"); it is the same from one session to the next, which the names are not.
    """
    names, out, info = {}, [], None
    for line in master_text.splitlines():
        if line.startswith("#EXT-X-MEDIA:"):
            g, n = re.search(r'GROUP-ID="([^"]*)"', line), re.search(r'NAME="([^"]*)"', line)
            if g and n:
                names[g.group(1)] = n.group(1)
        elif line.startswith("#EXT-X-STREAM-INF:"):
            info = line
        elif line and not line.startswith("#") and info:
            f = lambda rx: (re.search(rx, info) or [None, ""])[1]
            slug = f(r'VIDEO="([^"]*)"') or "v%d" % len(out)
            out.append({"slug": slug, "name": names.get(slug, slug),
                        "bandwidth": int(f(r"BANDWIDTH=(\d+)") or 0),
                        "resolution": f(r"RESOLUTION=(\d+x\d+)"), "fps": f(r"FRAME-RATE=([\d.]+)"),
                        "codecs": f(r'CODECS="([^"]*)"'), "url": line})
            info = None
    out.sort(key=lambda v: -v["bandwidth"])
    return out


def _when(text):
    try:
        return datetime.datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def parse_media(text):
    """A media playlist as data.

    {seq, target, segments: [{n, dur, title, pdt, url, ad, disc, map}], ads: {...} or None,
     odd_titles: [...], odd_classes: [...], client_side: bool}. `client_side` is True while Twitch is asking the player
    to show an advert of its own (see CLIENT_SIDE above); nothing is cut for it. `n` is Twitch's own sequence number for the segment. `ad` is True for
    advert segments. `map` is the address of the segment's init piece (#EXT-X-MAP) when the channel is
    sent as fMP4, which Twitch does for some channels since 2026 (5 of the 12 most watched on 2026-10-08),
    else None: a player cannot decode an fMP4 segment without it. `odd_titles` lists segment titles that are neither "live" nor a known advert
    shape: something new that somebody should look at. `odd_classes` does the same for DATERANGE classes
    (KNOWN_CLASSES above).
    """
    seq, target, segs, windows, odd, classes = 0, 6, [], [], set(), set()
    cue_out = False
    dur = title = pdt = None
    disc = False
    ads = None
    init = None
    for line in text.splitlines():
        if line.startswith("#EXT-X-MEDIA-SEQUENCE:"):
            seq = int(line.split(":", 1)[1])
        elif line.startswith("#EXT-X-TARGETDURATION:"):
            target = int(float(line.split(":", 1)[1]))
        elif line.startswith("#EXT-X-DATERANGE:") and AD_DATERANGE.search(line):
            a = lambda k: (re.search(r'%s="?([^",]*)' % k, line) or [None, ""])[1]
            start = _when(a("START-DATE"))
            length = float(a("DURATION") or a("X-TV-TWITCH-AD-POD-FILLED-DURATION") or 0)
            if start is not None and length:
                windows.append((start, start + length))
            ads = ads or {"roll": a("X-TV-TWITCH-AD-ROLL-TYPE").lower() or "advert",
                          "pod": a("X-TV-TWITCH-AD-POD-LENGTH"),
                          "seconds": a("X-TV-TWITCH-AD-POD-FILLED-DURATION") or a("DURATION")}
        elif line.startswith("#EXT-X-DATERANGE:"):
            found = re.search(r'CLASS="([^"]*)"', line)
            if found and found.group(1) not in KNOWN_CLASSES:
                classes.add(found.group(1)[:60])
        elif line.startswith("#EXT-X-CUE-OUT"):
            cue_out = True
        elif line.startswith("#EXT-X-CUE-IN"):
            cue_out = False
        elif line.startswith("#EXT-X-DISCONTINUITY") and not line.startswith("#EXT-X-DISCONTINUITY-SEQUENCE"):
            disc = True
        elif line.startswith("#EXT-X-MAP:"):
            found = re.search(r'URI="([^"]+)"', line)
            init = found.group(1) if found else None
        elif line.startswith("#EXT-X-PROGRAM-DATE-TIME:"):
            pdt = line.split(":", 1)[1]
        elif line.startswith("#EXTINF:"):
            d, _, title = line[8:].partition(",")
            dur = float(d)
        elif line and not line.startswith("#") and dur is not None:
            when = _when(pdt) if pdt else None
            known_ad = "|" in title or title.startswith(("Amazon", "DCM")) or cue_out or any(p in line for p in AD_URLS)
            in_window = when is not None and any(a - 0.05 <= when < b - 0.05 for a, b in windows)
            if not known_ad and title not in STREAM_TITLES:
                odd.add(title)
            segs.append({"n": seq + len(segs), "dur": dur, "title": title, "pdt": pdt, "when": when,
                         "url": line, "ad": known_ad or in_window, "disc": disc, "map": init})
            dur = title = pdt = None
            disc = False
    if ads is None and any(s["ad"] for s in segs):
        ads = {"roll": "advert", "pod": "", "seconds": ""}
    return {"seq": seq, "target": target, "segments": segs, "ads": ads, "odd_titles": sorted(odd),
            "odd_classes": sorted(classes), "client_side": CLIENT_SIDE in text}


def has_ads(media_text):
    m = parse_media(media_text)
    return bool(m["ads"]) or any(s["ad"] for s in m["segments"])


class Timeline:
    """The stream as we hand it on: stream segments only, numbered by us.

    Twitch numbers adverts and stream alike, so cutting adverts out of its playlist leaves holes a
    player trips over. feed() takes each fresh playlist and appends only the stream segments not
    seen before, giving them our own running number; render() writes the last `keep` of them.
    While a playlist is all adverts nothing is appended and render() keeps returning the same
    playlist: the player waits on the last picture instead of failing. A break where adverts were
    cut, or where the session changed, is marked with a discontinuity.
    """

    def __init__(self, keep=15):
        self.segs = collections.deque()
        self.keep = keep
        self.next_n = 0
        self.disc_base = 0
        self.target = 6
        self.last_when = None
        self.last_session = None
        self.gap = False
        self.seen = set()
        self.seen_order = collections.deque()
        self.cut = 0          # advert segments dropped, ever
        self.passed = 0       # stream segments passed on, ever

    def feed(self, session, media):
        """Returns (stream segments added, advert segments cut) for this playlist."""
        added = cut = 0
        self.target = max(self.target, media["target"])
        for s in media["segments"]:
            key = (session, s["n"])
            if key in self.seen:
                continue
            self.seen.add(key)
            self.seen_order.append(key)
            if len(self.seen_order) > 600:
                self.seen.discard(self.seen_order.popleft())
            if s["ad"]:
                cut += 1
                self.gap = True
                continue
            if self.last_when is not None and s["when"] is not None and s["when"] <= self.last_when + 0.05:
                continue      # a new session's playlist overlaps what was already handed on
            changed = self.last_session not in (None, session)
            disc = bool(self.segs) and (self.gap or changed or s["disc"])
            self.segs.append({"n": self.next_n, "dur": s["dur"], "pdt": s["pdt"], "url": s["url"], "disc": disc, "map": s.get("map")})
            self.next_n += 1
            self.gap = False
            self.last_session = session
            if s["when"] is not None:
                self.last_when = s["when"]
            added += 1
            while len(self.segs) > self.keep:
                if self.segs.popleft()["disc"]:
                    self.disc_base += 1
        self.cut += cut
        self.passed += added
        return added, cut

    def render(self):
        # fMP4 channels: each segment needs its init piece named first (#EXT-X-MAP, which needs version 6). It is
        # written before the first segment and again whenever it changes (a new session, or the stream after a cut).
        fmp4 = any(s.get("map") for s in self.segs)
        init = None
        out = ["#EXTM3U", "#EXT-X-VERSION:%d" % (6 if fmp4 else 3), "#EXT-X-TARGETDURATION:%d" % self.target,
               "#EXT-X-MEDIA-SEQUENCE:%d" % (self.segs[0]["n"] if self.segs else self.next_n),
               "#EXT-X-DISCONTINUITY-SEQUENCE:%d" % self.disc_base]
        for i, s in enumerate(self.segs):
            if s["disc"] and i:
                out.append("#EXT-X-DISCONTINUITY")
            elif s["disc"]:
                # the first segment in view starts a new run: already counted in disc_base + 1
                out[4] = "#EXT-X-DISCONTINUITY-SEQUENCE:%d" % (self.disc_base + 1)
            if s.get("map") and s["map"] != init:
                init = s["map"]
                out.append('#EXT-X-MAP:URI="%s"' % init)
            if s["pdt"]:
                out.append("#EXT-X-PROGRAM-DATE-TIME:" + s["pdt"])
            out.append("#EXTINF:%.3f,live" % s["dur"])
            out.append(s["url"])
        return "\n".join(out) + "\n"


def live(logins, via=None):
    """{login: {name, title, game, viewers}} for those of `logins` that are live now."""
    logins = [l.lower() for l in logins if valid_login(l)]
    if not logins:
        return {}
    q = """query($logins: [String!]) { users(logins: $logins) { login displayName
           stream { title viewersCount game { displayName } } } }"""
    out = {}
    for u in (gql(q, {"logins": logins}, via=via).get("users") or []):
        if u and u.get("stream"):
            s = u["stream"]
            out[u["login"]] = {"name": u["displayName"], "title": s.get("title") or "",
                               "viewers": s.get("viewersCount") or 0,
                               "game": (s.get("game") or {}).get("displayName") or ""}
    return out


def top(n=12, via=None):
    """The n most-watched live channels: [{login, name, title, game, viewers}]."""
    q = """query($n: Int!) { streams(first: $n) { edges { node { title viewersCount
           broadcaster { login displayName } game { displayName } } } } }"""
    out = []
    for e in (gql(q, {"n": n}, via=via).get("streams") or {}).get("edges") or []:
        s = e["node"]
        if s.get("broadcaster"):
            out.append({"login": s["broadcaster"]["login"], "name": s["broadcaster"]["displayName"],
                        "title": s.get("title") or "", "viewers": s.get("viewersCount") or 0,
                        "game": (s.get("game") or {}).get("displayName") or ""})
    return out
