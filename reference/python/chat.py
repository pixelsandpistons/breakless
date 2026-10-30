"""Twitch chat, read only and without an account, and the emotes people use in it.

Used by server.py (/api/chat/<channel>, /api/emotes/<channel>) and test_chat.py. Standard library only.
README.md section 12 says how it is wired and how to test it.

Chat. Twitch's chat is IRC. Anyone may read a channel's chat by joining as "justinfanNNNN" with no
password; such a guest cannot write. One connection (TLS, irc.chat.twitch.tv:6697) is kept per channel
somebody is reading here and dropped a minute after the last reader went away. Each line becomes a small
event with a running number; a page asks for "everything after number N" and the answer is held open up
to `wait` seconds until there is something (a long poll: it passes any proxy, which a kept-open stream
does not always do).

    {"n": 12, "k": "m", "id": "...", "login": "name", "name": "Name", "color": "#1E90FF", "text": "hello Kappa",
     "emotes": [["25", 6, 10]], "badges": ["moderator"], "me": false, "first": false, "reply": "", "bits": 0}
    {"n": 13, "k": "n", "text": "Name subscribed with Prime.", "said": "thanks"}      a notice of the channel
    {"n": 14, "k": "x", "login": "name"}      that person's messages were removed ("" = the whole chat was cleared)
    {"n": 15, "k": "d", "id": "..."}           one message was removed

`emotes` are Twitch's own, as [id, first character, last character] (counted in characters, not bytes).

Emotes. BetterTTV, FrankerFaceZ and 7TV each publish, without a key, a global list and a list per
channel (by the channel's Twitch number, which chat states when it is joined). emotes() merges them
into {word: [provider, id, zero_width]}; a page swaps the word for the picture. The pictures themselves
are fetched by whoever shows them (image_url() says from where).
"""
import collections
import json
import random
import re
import socket
import ssl
import threading
import time
import urllib.error
import urllib.request

IRC_HOST, IRC_PORT = "irc.chat.twitch.tv", 6697
KEEP = 200            # events remembered per channel
IDLE = 60             # seconds without a reader before a channel's connection is dropped
MAX_ROOMS = 12        # channels read at once; the one least recently read is dropped to make room
UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
      "Chrome/140.0.0.0 Safari/537.36")

# Badges worth showing as a word beside a name; the rest (bits, sub-gifter, hype train...) are left out.
BADGES = ("broadcaster", "moderator", "vip", "staff", "partner", "subscriber", "founder")

_UNESCAPE = {"s": " ", ":": ";", "\\": "\\", "r": "\r", "n": "\n"}


def log(*a):
    print(time.strftime("%H:%M:%S"), "chat:", *a, flush=True)


def tag_value(v):
    """IRCv3 tag values escape space, semicolon and backslash."""
    if "\\" not in v:
        return v
    return re.sub(r"\\(.?)", lambda m: _UNESCAPE.get(m.group(1), m.group(1)), v)


def parse_line(line):
    """One IRC line as (tags, prefix, command, params). params' last item is the trailing text."""
    tags, prefix = {}, ""
    if line.startswith("@"):
        raw, _, line = line[1:].partition(" ")
        for pair in raw.split(";"):
            k, _, v = pair.partition("=")
            tags[k] = tag_value(v)
    if line.startswith(":"):
        prefix, _, line = line[1:].partition(" ")
    head, sep, trailing = line.partition(" :")
    params = head.split()
    command = params.pop(0) if params else ""
    if sep:
        params.append(trailing)
    return tags, prefix, command, params


def parse_emotes(spec):
    """"25:0-4,12-16/1902:6-10" -> [["25", 0, 4], ["1902", 6, 10], ["25", 12, 16]], by position."""
    out = []
    for part in (spec or "").split("/"):
        eid, _, ranges = part.partition(":")
        if not re.fullmatch(r"[A-Za-z0-9_]{1,80}", eid):
            continue
        for r in ranges.split(","):
            m = re.fullmatch(r"(\d+)-(\d+)", r)
            if m and int(m.group(1)) <= int(m.group(2)):
                out.append([eid, int(m.group(1)), int(m.group(2))])
    out.sort(key=lambda e: e[1])
    return out


def event(tags, prefix, command, params):
    """An IRC line as one of the events described at the top, or None for a line that is not one."""
    if command == "PRIVMSG" and len(params) >= 2:
        text, me, shift = params[1], False, 0
        if text.startswith("\x01ACTION ") and text.endswith("\x01"):      # "/me ..."
            text, me, shift = text[8:-1], True, 8
        login = prefix.split("!", 1)[0]
        return {"k": "m", "id": tags.get("id", ""), "login": login, "name": tags.get("display-name") or login,
                "color": tags.get("color", "") if re.fullmatch(r"#[0-9A-Fa-f]{6}", tags.get("color", "")) else "",
                "text": text[:600], "emotes": [[i, a - shift, b - shift] for i, a, b in parse_emotes(tags.get("emotes"))],
                "badges": [b for b in (x.split("/", 1)[0] for x in tags.get("badges", "").split(",")) if b in BADGES],
                "me": me, "first": tags.get("first-msg") == "1", "reply": tags.get("reply-parent-display-name", ""),
                "bits": int(tags["bits"]) if tags.get("bits", "").isdigit() else 0}
    if command == "USERNOTICE":
        text = tags.get("system-msg", "")
        if not text:
            return None
        return {"k": "n", "text": text[:300], "said": (params[1] if len(params) >= 2 else "")[:300]}
    if command == "CLEARCHAT":
        return {"k": "x", "login": params[1] if len(params) >= 2 else ""}
    if command == "CLEARMSG":
        return {"k": "d", "id": tags.get("target-msg-id", "")}
    return None


class Room:
    """One channel's chat: the connection, the last KEEP events, and the people waiting for the next one."""

    def __init__(self, login, connect=True):
        self.login = login
        self.events = collections.deque(maxlen=KEEP)
        self.n = 0
        self.cond = threading.Condition()
        self.used = time.time()
        self.room_id = ""
        self.state = "connecting"     # connecting | joined | down
        self.closed = False
        self.sock = None
        if connect:
            threading.Thread(target=self.run, daemon=True, name="chat-" + login).start()

    def add(self, ev):
        with self.cond:
            self.n += 1
            ev["n"] = self.n
            self.events.append(ev)
            self.cond.notify_all()

    def take(self, tags, prefix, command, params):
        """What one parsed line does to the room. Returns a line to answer with, or None."""
        if command == "PING":
            return "PONG :" + (params[-1] if params else "tmi.twitch.tv")
        if command == "ROOMSTATE" and tags.get("room-id"):
            self.room_id = tags["room-id"]
            self.state = "joined"
        elif command == "RECONNECT":
            raise ConnectionError("Twitch asked for a reconnect")
        else:
            ev = event(tags, prefix, command, params)
            if ev:
                self.add(ev)
        return None

    def run(self):
        wait = 1
        while not self.closed:
            try:
                raw = socket.create_connection((IRC_HOST, IRC_PORT), timeout=15)
                s = self.sock = ssl.create_default_context().wrap_socket(raw, server_hostname=IRC_HOST)
                s.settimeout(330)      # Twitch pings every five minutes or so; silence longer than that is a dead line
                s.sendall(("CAP REQ :twitch.tv/tags twitch.tv/commands\r\nNICK justinfan%d\r\nJOIN #%s\r\n"
                           % (random.randint(10000, 999999), self.login)).encode())
                buf = b""
                while not self.closed:
                    data = s.recv(8192)
                    if not data:
                        raise ConnectionError("closed by Twitch")
                    buf += data
                    *lines, buf = buf.split(b"\r\n")
                    for line in lines:
                        answer = self.take(*parse_line(line.decode("utf-8", "replace")))
                        if answer:
                            s.sendall((answer + "\r\n").encode())
                    wait = 1
            except (OSError, ConnectionError, ssl.SSLError) as e:
                if self.closed:
                    break
                self.state = "down"
                log("%s: %s; again in %ds" % (self.login, str(e)[:80], wait))
                time.sleep(wait)
                wait = min(wait * 2, 30)
            finally:
                try:
                    if self.sock:
                        self.sock.close()
                except OSError:
                    pass

    def close(self):
        self.closed = True
        try:
            if self.sock:
                self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        with self.cond:
            self.cond.notify_all()

    def poll(self, after=None, wait=0.0):
        """Events numbered above `after` (None: the last 60), waiting up to `wait` seconds for the first."""
        self.used = time.time()
        deadline = time.time() + max(0.0, min(float(wait), 25.0))
        with self.cond:
            if after is None or after > self.n:      # a page that knows a number we never gave: the service restarted
                out = list(self.events)[-60:]
            else:
                while self.n <= after and not self.closed:
                    left = deadline - time.time()
                    if left <= 0:
                        break
                    self.cond.wait(left)
                out = [e for e in self.events if e["n"] > after]
            self.used = time.time()
            return {"channel": self.login, "seq": self.n, "state": self.state, "room_id": self.room_id, "events": out}


ROOMS = {}
ROOMS_LOCK = threading.Lock()
_reaping = False


def room(login):
    global _reaping
    login = login.lower()
    with ROOMS_LOCK:
        if not _reaping:
            _reaping = True
            threading.Thread(target=_reaper, daemon=True, name="chat-reaper").start()
        r = ROOMS.get(login)
        if r is None:
            if len(ROOMS) >= MAX_ROOMS:
                oldest = min(ROOMS.values(), key=lambda x: x.used)
                oldest.close()
                del ROOMS[oldest.login]
            r = ROOMS[login] = Room(login)
            log("%s: joining" % login)
        return r


def _reaper():
    while True:
        time.sleep(15)
        with ROOMS_LOCK:
            for login in [k for k, r in ROOMS.items() if time.time() - r.used > IDLE]:
                ROOMS[login].close()
                del ROOMS[login]
                log("%s: nobody reading, left" % login)


def status():
    with ROOMS_LOCK:
        return [{"channel": r.login, "state": r.state, "events": r.n, "idle_seconds": round(time.time() - r.used, 1)}
                for r in ROOMS.values()]


# -- emotes -------------------------------------------------------------------------------------
# provider letter -> where one picture is, at twice the size chat shows it (sharp on a dense screen)
IMAGES = {
    "t": "https://static-cdn.jtvnw.net/emoticons/v2/%s/default/dark/2.0",      # Twitch's own
    "b": "https://cdn.betterttv.net/emote/%s/2x.webp",
    "f": "https://cdn.frankerfacez.com/emote/%s/2",
    "7": "https://cdn.7tv.app/emote/%s/2x.webp",
}
NAMES = {"b": "BetterTTV", "f": "FrankerFaceZ", "7": "7TV"}
GLOBAL_AGE, CHANNEL_AGE = 3600, 600
_cache = {}
_cache_lock = threading.Lock()


def image_url(provider, eid):
    """Where an emote's picture is, or None for a provider or an id that is not one."""
    if provider not in IMAGES or not re.fullmatch(r"[A-Za-z0-9_]{1,80}", eid or ""):
        return None
    return IMAGES[provider] % eid


def _json(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=6) as r:
        return json.loads(r.read(4_000_000).decode("utf-8", "replace"))


def from_bttv(data):
    """BetterTTV's answer (a list, or {channelEmotes, sharedEmotes}) as {word: ["b", id, zero_width]}."""
    rows = data if isinstance(data, list) else (data.get("channelEmotes") or []) + (data.get("sharedEmotes") or [])
    return {e["code"]: ["b", e["id"], bool(e.get("modifier"))] for e in rows
            if isinstance(e, dict) and e.get("code") and image_url("b", str(e.get("id", "")))}


def from_ffz(data):
    """FrankerFaceZ's answer ({sets: {id: {emoticons}}}, global ones limited to default_sets)."""
    sets = data.get("sets") or {}
    wanted = [str(i) for i in data["default_sets"]] if "default_sets" in data else list(sets)
    out = {}
    for sid in wanted:
        for e in (sets.get(sid) or {}).get("emoticons") or []:
            if e.get("name") and image_url("f", str(e.get("id", ""))):
                out[e["name"]] = ["f", str(e["id"]), bool(e.get("modifier"))]
    return out


def from_7tv(data):
    """7TV's answer (a set, or a user with emote_set). flags & 1 on an emote means it is drawn over the one before."""
    rows = (data.get("emote_set") or {}).get("emotes") if "emote_set" in data else data.get("emotes")
    return {e["name"]: ["7", e["id"], bool(int(e.get("flags") or 0) & 1)] for e in rows or []
            if isinstance(e, dict) and e.get("name") and image_url("7", str(e.get("id", "")))}


SOURCES = [      # (provider, read, global address, channel address); later rows win a word two providers both have
    ("f", from_ffz, "https://api.frankerfacez.com/v1/set/global", "https://api.frankerfacez.com/v1/room/id/%s"),
    ("b", from_bttv, "https://api.betterttv.net/3/cached/emotes/global", "https://api.betterttv.net/3/cached/users/twitch/%s"),
    ("7", from_7tv, "https://7tv.io/v3/emote-sets/global", "https://7tv.io/v3/users/twitch/%s"),
]


def _cached(url, read, age, fetch):
    now = time.time()
    with _cache_lock:
        hit = _cache.get(url)
    if hit and now - hit[0] < age:
        return hit[1], hit[2]
    try:
        got, err = read(fetch(url)), ""
    except urllib.error.HTTPError as e:
        got, err = {}, ("" if e.code == 404 else "HTTP %s" % e.code)      # 404: the channel has no list there. Not a fault
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:
        got, err = (hit[1] if hit else {}), str(e)[:80]                   # a provider that is away: keep what we had
    with _cache_lock:
        _cache[url] = (now if not err else now - age + 60, got, err)      # a failure is asked again in a minute
        if len(_cache) > 200:
            for k in sorted(_cache, key=lambda k: _cache[k][0])[:50]:
                del _cache[k]
    return got, err


def emotes(room_id, fetch=_json):
    """{"emotes": {word: [provider, id, zero_width]}, "counts": {"b": {"global": n, "channel": n}, ...}, "errors": {}}.

    Global lists first, then the channel's own, which win. room_id "" (chat not joined yet) gives the globals alone.
    """
    out, counts, errors = {}, {}, {}
    scopes = ("global", "channel") if re.fullmatch(r"\d{1,20}", room_id or "") else ("global",)
    jobs = [(scope, letter, read, world if scope == "global" else mine % room_id, GLOBAL_AGE if scope == "global" else CHANNEL_AGE)
            for scope in scopes for letter, read, world, mine in SOURCES]
    answers = {}

    def ask(job):
        answers[job[:2]] = _cached(job[3], job[2], job[4], fetch)
    threads = [threading.Thread(target=ask, args=(j,), daemon=True) for j in jobs]      # six lists at once: one slow
    for th in threads:                                                                  # provider does not hold the rest
        th.start()
    for th in threads:
        th.join(12)
    for scope, letter, *_ in jobs:
        got, err = answers.get((scope, letter), ({}, "no answer in time"))
        out.update(got)
        counts.setdefault(letter, {"global": 0, "channel": 0})[scope] = len(got)
        if err:
            errors[NAMES[letter] + " " + scope] = err
    return {"room_id": room_id or "", "emotes": out, "counts": counts, "errors": errors}
