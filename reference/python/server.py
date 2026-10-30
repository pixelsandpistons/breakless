#!/usr/bin/env python3
"""Twitch without adverts: hands any player a clean playlist for a live channel.

    GET /                         the page (static/): channel list, player, chat
    GET /live/<channel>.m3u8      master playlist; open it in mpv, VLC, a phone, a TV
    GET /live/<channel>/<q>.m3u8  one quality, adverts cut out, numbered by us
    GET /api/channels             followed channels and the most watched, with who is live
    POST /api/follow              {"login": "name", "on": true|false}
    GET /api/status               the exit, who is being watched, adverts seen and cut
    GET /api/watch/<channel>      the part of /api/status one viewer wants: the exit, this channel's advert break
    GET /api/chat/<channel>?after=N&wait=S    chat, read only: the events numbered above N, held open up to S
                                  seconds (25 at most) until there is one. Without `after`: the last 60. chat.py
    GET /api/emotes/<channel>     the BetterTTV, FrankerFaceZ and 7TV emotes of that chat: {word: [provider, id, zero_width]}
    GET /health                   200 while the process is up

How it works, what was measured and what is left: README.md beside this file.
Everything that knows Twitch's formats is in twitchlib.py. Standard library only.

Environment:
    TWITCH_PROXY   HTTP proxy in a country without Twitch adverts (http://twitch-vpn:8888).
                   Empty: ask from the home line and cut the adverts out.
    TWITCH_HOST    the name this is served under (twitch.<domain>), for the chat frame.
    TWITCH_DATA    where follows.json and adverts.jsonl live (default /data).
    TWITCH_PORT    default 8080.
    TWITCH_BIND    default 0.0.0.0 (right inside the container). A copy run on the host for a test
                   must use 127.0.0.1 (host/security/SECURITY.md) and be stopped afterwards.
"""
import json
import os
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import twitchlib as t  # noqa: E402
import chat  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
PROXY = os.environ.get("TWITCH_PROXY", "").strip()
HOST = os.environ.get("TWITCH_HOST", "").strip()
DATA = os.environ.get("TWITCH_DATA", "/data")
PORT = int(os.environ.get("TWITCH_PORT", "8080"))
BIND = os.environ.get("TWITCH_BIND", "0.0.0.0")

IDLE = 90            # seconds without a request before a channel's session is dropped
REFRESH = 0.9        # seconds a quality's playlist is reused before asking Twitch again
PROXY_REST = 30      # seconds the exit is left alone after it failed
STARTED = time.time()

EXIT, HOME = "exit", "home"     # where a session's playlists are asked from


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


class Offline(Exception):
    pass


class Exit:
    """The proxy in the advert-free country, and whether it is answering."""
    via = t.opener(PROXY) if PROXY else None
    down_until = 0.0
    last_error = ""
    last_ok = 0.0
    country = ""
    cluster = ""

    @classmethod
    def usable(cls):
        return cls.via is not None and time.time() >= cls.down_until

    @classmethod
    def failed(cls, why):
        cls.down_until = time.time() + PROXY_REST
        cls.last_error = "%s %s" % (time.strftime("%Y-%m-%d %H:%M:%S"), why)
        log("exit failed, using the home line for %ds: %s" % (PROXY_REST, why))


class Totals:
    lock = threading.Lock()
    breaks = {EXIT: 0, HOME: 0}       # advert breaks met, by where the playlist was asked from
    client_side = 0                   # times Twitch asked the player to show its own advert (ignored)
    cut = {EXIT: 0, HOME: 0}          # advert segments cut out
    passed = 0                        # stream segments handed on
    sessions = {EXIT: 0, HOME: 0}
    odd_titles = set()
    odd_classes = set()


def record(event):
    """One line per advert break in adverts.jsonl, so anyone can see what got through where."""
    event["at"] = time.strftime("%Y-%m-%d %H:%M:%S")
    try:
        path = os.path.join(DATA, "adverts.jsonl")
        if os.path.exists(path) and os.path.getsize(path) > 1_000_000:
            os.replace(path, path + ".1")
        with open(path, "a") as f:
            f.write(json.dumps(event) + "\n")
    except OSError as e:
        log("cannot write adverts.jsonl:", e)


class Channel:
    def __init__(self, login):
        self.login = login
        self.lock = threading.Lock()
        self.session = None
        self.sessions_opened = 0
        self.timelines = {}
        self.fetched = {}
        self.used = time.time()
        self.in_break = None       # {"since", "roll", "seconds", "cut"} while adverts are being cut
        self.retry_exit = 0.0
        self.client_side = False

    # -- sessions ---------------------------------------------------------------------------
    def open(self):
        """Ask Twitch for a master playlist: through the exit if it answers, else from home."""
        # The exit is asked twice before the home line is used: about one request in a few hundred dies in
        # transit (SSL EOF, a timeout), and falling back for that costs the viewer a pre-roll's worth of
        # frozen picture (seen 2026-10-10: one dropped connection, 36 s of waiting).
        attempts = ([(EXIT, Exit.via)] * 2 if Exit.usable() else []) + [(HOME, None)]
        last = None
        for i, (mode, via) in enumerate(attempts):
            try:
                text = t.master(self.login, via=via)
            except t.TwitchError as e:
                last = e
                # usher answers 404 for a channel that is not live; no token at all means no such channel
                if getattr(e, "code", 0) == 404 or str(e).startswith("no token"):
                    raise Offline(self.login)
                if mode == EXIT and attempts[i + 1][0] == EXIT:
                    log("%s: the exit did not answer (%s), asking it once more" % (self.login, str(e)[:80]))
                elif mode == EXIT:
                    Exit.failed(str(e)[:120])
                continue
            v = t.variants(text)
            if not v:
                last = t.TwitchError("no qualities in the master playlist")
                continue
            info = t.master_info(text)
            self.sessions_opened += 1
            self.session = {"id": "%s-%d" % (self.login, self.sessions_opened), "mode": mode, "via": via,
                            "variants": {x["slug"]: x for x in v}, "order": [x["slug"] for x in v],
                            "country": info["country"], "cluster": info["cluster"], "opened": time.time()}
            with Totals.lock:
                Totals.sessions[mode] += 1
            if mode == EXIT:
                Exit.last_ok, Exit.country, Exit.cluster = time.time(), info["country"], info["cluster"]
            log("%s: session from %s, Twitch sees %s, %d qualities" % (self.login, mode, info["country"], len(v)))
            return self.session
        raise last or t.TwitchError("no session")

    def master(self):
        with self.lock:
            self.used = time.time()
            s = self.session or self.open()
            out = ["#EXTM3U"]
            for slug in s["order"]:
                v = s["variants"][slug]
                out.append('#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="%s",NAME="%s",AUTOSELECT=YES,DEFAULT=YES' % (slug, v["name"]))
                inf = "#EXT-X-STREAM-INF:BANDWIDTH=%d" % v["bandwidth"]
                if v["resolution"]:
                    inf += ",RESOLUTION=" + v["resolution"]
                if v["codecs"]:
                    inf += ',CODECS="%s"' % v["codecs"]
                inf += ',VIDEO="%s"' % slug
                if v["fps"]:
                    inf += ",FRAME-RATE=" + v["fps"]
                out += [inf, "%s/%s.m3u8" % (self.login, slug)]
            return "\n".join(out) + "\n"

    # -- one quality ------------------------------------------------------------------------
    def media(self, slug):
        with self.lock:
            now = self.used = time.time()
            line = self.timelines.setdefault(slug, t.Timeline())
            if now - self.fetched.get(slug, 0) < REFRESH:
                return line.render()
            self.fetched[slug] = now
            # A session that had to be opened from home goes back to the exit as soon as it answers.
            if self.session and self.session["mode"] == HOME and Exit.usable() and now >= self.retry_exit:
                self.retry_exit = now + PROXY_REST
                self.session = None
            parsed = None
            for attempt in (1, 2, 3):
                try:
                    s = self.session or self.open()
                    v = s["variants"].get(slug) or s["variants"][s["order"][0]]
                    parsed = t.parse_media(t.http(v["url"], via=s["via"]))
                    break
                except Offline:
                    raise
                except t.TwitchError as e:
                    passing = getattr(e, "code", 0) in (0, 502, 503, 504)
                    log("%s/%s: %s" % (self.login, slug, str(e)[:100]))
                    # About one request in a few hundred through the exit dies in transit
                    # (seen 2026-10-04: SSL EOF). Ask once more before giving the session up.
                    if passing and attempt == 1 and self.session:
                        continue
                    # Otherwise the session is over: Twitch said so (403/404), or the exit is away.
                    if passing and self.session and self.session["mode"] == EXIT:
                        Exit.failed(str(e)[:120])
                    self.session = None
                    if attempt == 3 and not line.segs:
                        raise
            if parsed is None:
                return line.render()
            s = self.session
            added, cut = line.feed(s["id"], parsed)
            with Totals.lock:
                Totals.passed += added
                Totals.cut[s["mode"]] += cut
                Totals.odd_titles.update(parsed["odd_titles"])
                Totals.odd_classes.update(parsed["odd_classes"])
            self.note_adverts(s, parsed, cut)
            return line.render()

    def note_adverts(self, s, parsed, cut):
        now = time.time()
        if parsed["client_side"] != self.client_side:
            self.client_side = parsed["client_side"]
            if self.client_side:
                with Totals.lock:
                    Totals.client_side += 1
                log("%s: Twitch asks the player to show an advert of its own; ignored, stream untouched" % self.login)
        # A break lasts while the newest segment is an advert. (The advert's DATERANGE tag stays
        # in the playlist for half a minute after the stream is back, so it cannot be the test.)
        if parsed["ads"] and parsed["segments"] and parsed["segments"][-1]["ad"]:
            if not self.in_break:
                self.in_break = {"since": now, "roll": parsed["ads"]["roll"], "seconds": parsed["ads"]["seconds"],
                                 "cut": 0, "mode": s["mode"], "country": s["country"]}
                with Totals.lock:
                    Totals.breaks[s["mode"]] += 1
                log("%s: advert break (%s, %ss) on a %s session, cutting" % (
                    self.login, parsed["ads"]["roll"], parsed["ads"]["seconds"] or "?", s["mode"]))
            self.in_break["cut"] += cut
        elif self.in_break:
            b, self.in_break = self.in_break, None
            record({"channel": self.login, "from": b["mode"], "country": b["country"], "roll": b["roll"],
                    "advertised_seconds": b["seconds"], "held_seconds": round(now - b["since"], 1),
                    "segments_cut": b["cut"]})
            log("%s: advert break over after %.0fs, %d segments cut" % (self.login, now - b["since"], b["cut"]))

    def status(self):
        s = self.session
        return {"channel": self.login, "from": s["mode"] if s else None, "country": s["country"] if s else None,
                "qualities": [s["variants"][q]["name"] for q in s["order"]] if s else [],
                "asked": sorted(self.timelines), "idle_seconds": round(time.time() - self.used, 1),
                "advert_break": ({"roll": self.in_break["roll"], "seconds": self.in_break["seconds"],
                                  "for_seconds": round(time.time() - self.in_break["since"], 1)}
                                 if self.in_break else None),
                "segments_cut": sum(l.cut for l in self.timelines.values()),
                "segments_passed": sum(l.passed for l in self.timelines.values())}


CHANNELS = {}
CHANNELS_LOCK = threading.Lock()


def channel(login):
    login = login.lower()
    with CHANNELS_LOCK:
        if login not in CHANNELS:
            CHANNELS[login] = Channel(login)
        return CHANNELS[login]


def reaper():
    while True:
        time.sleep(15)
        with CHANNELS_LOCK:
            for login in [k for k, c in CHANNELS.items() if time.time() - c.used > IDLE]:
                log("%s: nobody watching, session dropped" % login)
                del CHANNELS[login]


# -- the channel list ---------------------------------------------------------------------------
FOLLOWS_LOCK = threading.Lock()
LISTING = {"at": 0.0, "data": None}


def follows():
    try:
        with open(os.path.join(DATA, "follows.json")) as f:
            return [x for x in json.load(f) if t.valid_login(x)]
    except (OSError, ValueError):
        return []


def set_follow(login, on):
    with FOLLOWS_LOCK:
        cur = [x for x in follows() if x != login]
        if on:
            cur.append(login)
        tmp = os.path.join(DATA, "follows.json.tmp")
        with open(tmp, "w") as f:
            json.dump(cur, f)
        os.replace(tmp, os.path.join(DATA, "follows.json"))
        LISTING["at"] = 0.0


def listing():
    """Followed channels (live first) and the most watched. Asked from Twitch twice a minute at most."""
    if LISTING["data"] is not None and time.time() - LISTING["at"] < 30:
        return LISTING["data"]
    mine = follows()
    out = {"follows": [], "top": [], "error": None}
    try:
        on = t.live(mine)
        out["follows"] = sorted(({"login": l, "live": l in on, **on.get(l, {"name": l})} for l in mine),
                                key=lambda c: (not c["live"], -c.get("viewers", 0), c["login"]))
        out["top"] = t.top(12)
    except (t.TwitchError, ValueError, KeyError, TypeError) as e:
        out["follows"] = [{"login": l, "live": None, "name": l} for l in mine]
        out["error"] = str(e)[:200]
    LISTING.update(at=time.time(), data=out)
    return out


def status():
    with CHANNELS_LOCK:
        watching = [c.status() for c in CHANNELS.values() if c.session]
    with Totals.lock:
        totals = {"advert_breaks": dict(Totals.breaks), "advert_segments_cut": dict(Totals.cut),
                  "client_side_advert_requests_ignored": Totals.client_side,
                  "stream_segments_passed": Totals.passed, "sessions": dict(Totals.sessions),
                  "segment_titles_not_seen_before": sorted(Totals.odd_titles),
                  "daterange_classes_not_seen_before": sorted(Totals.odd_classes)}
    return {"up_seconds": round(time.time() - STARTED),
            "exit": {"configured": bool(PROXY), "usable": Exit.usable(), "country": Exit.country,
                     "cluster": Exit.cluster, "last_ok_seconds_ago": round(time.time() - Exit.last_ok) if Exit.last_ok else None,
                     "last_error": Exit.last_error},
            "watching": watching, "chat": chat.status(), "totals": totals}


def watch_status(login):
    """What one viewer's player shows: where the playlists are asked from and whether adverts are being cut now."""
    with CHANNELS_LOCK:
        c = CHANNELS.get(login.lower())
    mine = c.status() if c and c.session else None
    with Totals.lock:
        cut = Totals.cut[EXIT] + Totals.cut[HOME]
    return {"channel": login.lower(), "exit": {"configured": bool(PROXY), "usable": Exit.usable(), "country": Exit.country},
            "from": mine["from"] if mine else None, "country": mine["country"] if mine else None,
            "advert_break": mine["advert_break"] if mine else None,
            "segments_cut": mine["segments_cut"] if mine else 0, "segments_cut_since_start": cut}


# -- HTTP ---------------------------------------------------------------------------------------
TYPES = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
         ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml", ".png": "image/png", ".ico": "image/x-icon"}
CSP = ("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https://static-cdn.jtvnw.net; "
       "media-src 'self' blob: https:; connect-src 'self' https:; worker-src 'self' blob:; "
       "frame-src https://www.twitch.tv; frame-ancestors 'self'; base-uri 'none'; form-action 'self'")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "twitch"

    def log_message(self, fmt, *args):
        pass

    def send(self, code, body, ctype="application/json", cache="no-store"):
        if isinstance(body, (dict, list)):
            body = json.dumps(body)
        if isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", cache)
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        if ctype.startswith("text/html"):
            self.send_header("Content-Security-Policy", CSP)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        path, _, query = self.path.partition("?")
        try:
            if path == "/health":
                return self.send(200, "ok\n", "text/plain")
            if path == "/api/status":
                return self.send(200, status())
            if path == "/api/channels":
                return self.send(200, dict(listing(), host=HOST))
            m = re.fullmatch(r"/api/watch/([A-Za-z0-9_]{2,25})", path)
            if m:
                return self.send(200, watch_status(m.group(1)))
            m = re.fullmatch(r"/api/chat/([A-Za-z0-9_]{2,25})", path)
            if m:
                q = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
                after = int(q["after"]) if q.get("after", "").isdigit() else None
                try:
                    wait = float(q.get("wait", "0"))
                except ValueError:
                    wait = 0.0
                return self.send(200, chat.room(m.group(1)).poll(after, wait))
            m = re.fullmatch(r"/api/emotes/([A-Za-z0-9_]{2,25})", path)
            if m:
                r = chat.room(m.group(1))
                for _ in range(20):                 # the channel's number comes with the join: give it two seconds
                    if r.room_id:
                        break
                    time.sleep(0.1)
                return self.send(200, chat.emotes(r.room_id), cache="private, max-age=300")
            m = re.fullmatch(r"/live/([A-Za-z0-9_]{2,25})\.m3u8", path)
            if m:
                return self.send(200, channel(m.group(1)).master(), "application/vnd.apple.mpegurl")
            m = re.fullmatch(r"/live/([A-Za-z0-9_]{2,25})/([A-Za-z0-9_]{1,20})\.m3u8", path)
            if m:
                return self.send(200, channel(m.group(1)).media(m.group(2)), "application/vnd.apple.mpegurl")
            if path == "/" or re.fullmatch(r"/watch/[A-Za-z0-9_]{2,25}", path):
                path = "/static/index.html"
            m = re.fullmatch(r"/static/([A-Za-z0-9_.-]+)", path)
            if m and ".." not in m.group(1):
                f = os.path.join(HERE, "static", m.group(1))
                if os.path.isfile(f):
                    with open(f, "rb") as fh:
                        return self.send(200, fh.read(), TYPES.get(os.path.splitext(f)[1], "application/octet-stream"),
                                         "no-cache")
            return self.send(404, {"error": "not found"})
        except Offline as e:
            return self.send(404, {"error": "%s is not live" % e})
        except t.TwitchError as e:
            return self.send(502, {"error": str(e)[:200]})
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_POST(self):
        # Read the body before anything else: the connection is kept open, and an unread body
        # would be taken for the start of the next request.
        try:
            length = int(self.headers.get("Content-Length", 0))
        except ValueError:
            length = 0
        if length > 2000:
            self.close_connection = True
            return self.send(413, {"error": "too long"})
        raw = self.rfile.read(length)
        # Only this page may change the list: the browser states where the request came from.
        origin = self.headers.get("Origin", "")
        if origin and origin.split("://", 1)[-1] != self.headers.get("Host", ""):
            return self.send(403, {"error": "cross-site request refused"})
        try:
            body = json.loads(raw or b"{}")
            if not isinstance(body, dict):
                raise ValueError
        except ValueError:
            return self.send(400, {"error": "not JSON"})
        if self.path == "/api/follow":
            login = str(body.get("login", "")).strip().lower()
            if not t.valid_login(login):
                return self.send(400, {"error": "a channel name is 2 to 25 letters, digits or _"})
            try:
                set_follow(login, bool(body.get("on", True)))
            except OSError as e:
                return self.send(500, {"error": "cannot write the channel list: %s" % e})
            return self.send(200, {"ok": True, "follows": follows()})
        return self.send(404, {"error": "not found"})


def main():
    threading.Thread(target=reaper, daemon=True).start()
    log("listening on %d; exit %s; data in %s" % (PORT, PROXY or "not set (home line, adverts cut)", DATA))
    ThreadingHTTPServer((BIND, PORT), Handler).serve_forever()


if __name__ == "__main__":
    main()
