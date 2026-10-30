// The page: lists from /api/channels, the player on /live/<channel>.m3u8, the lamp from /api/status.
// hls.min.js is hls.js 1.5.17, kept here so the page needs nothing from outside but Twitch's video.
(function () {
  'use strict';
  var $ = function (id) { return document.getElementById(id); };
  var video = $('video'), hls = null, current = null, listing = { follows: [], top: [], host: '' };

  function el(tag, attrs, kids) {
    var n = document.createElement(tag);
    Object.keys(attrs || {}).forEach(function (k) {
      if (k === 'text') n.textContent = attrs[k]; else n.setAttribute(k, attrs[k]);
    });
    (kids || []).forEach(function (k) { n.appendChild(k); });
    return n;
  }

  function people(n) { return n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1) + 'K' : String(n); }

  function card(c) {
    var picture = c.live === false || c.live === null
      ? el('div', { class: 'off', text: c.live === null ? 'not known' : 'offline' })
      : el('img', { loading: 'lazy', alt: '', referrerpolicy: 'no-referrer',
          src: 'https://static-cdn.jtvnw.net/previews-ttv/live_user_' + c.login + '-440x248.jpg' });
    var text = el('div', { class: 't' }, [
      el('b', { text: c.name || c.login }),
      el('small', { text: c.title || ' ' }),
      el('small', {}, c.viewers !== undefined
        ? [el('span', { class: 'n', text: people(c.viewers) + ' watching' }), document.createTextNode(c.game ? ' · ' + c.game : '')]
        : [document.createTextNode(' ')])
    ]);
    return el('a', { class: 'card', href: '/watch/' + c.login }, [picture, text]);
  }

  function fill(id, items) {
    var box = $(id);
    box.textContent = '';
    items.forEach(function (c) { box.appendChild(card(c)); });
  }

  function loadLists() {
    fetch('/api/channels').then(function (r) { return r.json(); }).then(function (d) {
      listing = d;
      fill('follows', d.follows);
      fill('top', d.top);
      $('none').hidden = d.follows.length > 0;
      if (current) describe();
    }).catch(function () {});
  }

  function known(login) {
    return listing.follows.concat(listing.top).filter(function (c) { return c.login === login; })[0];
  }

  function describe() {
    var c = known(current) || {};
    $('who').textContent = c.name || current;
    $('what').textContent = [c.game, c.title].filter(Boolean).join(' · ');
    var mine = listing.follows.some(function (f) { return f.login === current; });
    $('follow').textContent = mine ? 'Following' : 'Follow';
    $('follow').className = mine ? 'on' : '';
    document.title = (c.name || current) + ' · Twitch, no adverts';
  }

  function notice(text) {
    $('notice').hidden = !text;
    $('notice').textContent = text || '';
  }

  function stop() {
    if (hls) { hls.destroy(); hls = null; }
    video.removeAttribute('src');
    video.load();
  }

  function watch(login) {
    stop();
    current = login.toLowerCase();
    $('watch').hidden = false;
    notice('');
    describe();
    var url = '/live/' + current + '.m3u8';
    var q = $('quality');
    q.textContent = '';
    if (window.Hls && Hls.isSupported()) {
      hls = new Hls({ liveSyncDurationCount: 3, manifestLoadingMaxRetry: 2, levelLoadingMaxRetry: 8 });
      hls.on(Hls.Events.MANIFEST_PARSED, function (e, data) {
        q.appendChild(el('option', { value: '-1', text: 'Auto' }));
        data.levels.forEach(function (l, i) {
          q.appendChild(el('option', { value: String(i), text: l.name || (l.height ? l.height + 'p' : 'audio') }));
        });
        // start on the best picture: this is a home line, not a phone plan
        var best = 0;
        data.levels.forEach(function (l, i) { if (l.bitrate > data.levels[best].bitrate) best = i; });
        var want = localStorage.getItem('quality');
        hls.currentLevel = want === 'auto' ? -1 : best;
        q.value = String(want === 'auto' ? -1 : best);
        video.play().catch(function () {});
      });
      hls.on(Hls.Events.FRAG_BUFFERED, function () {
        if (/Lost the stream/.test($('notice').textContent)) notice('');
      });
      hls.on(Hls.Events.ERROR, function (e, data) {
        if (!data.fatal) return;
        if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
          // 404 with our own "is not live" means the stream ended. Any other failure (the service
          // restarting answers 404 from Traefik for a few seconds) is worth waiting out.
          var was = current;
          fetch(url).then(function (r) { return r.status === 404 ? r.json() : {}; }).catch(function () { return {}; }).then(function (j) {
            if (was !== current || !hls) return;
            if (j && j.error && /not live/.test(j.error)) {
              notice(((known(current) || {}).name || current) + ' is not live.');
              stop();
            } else {
              notice('Lost the stream, trying again.');
              setTimeout(function () { if (hls && was === current) { hls.loadSource(url); hls.startLoad(); } }, 3000);
            }
          });
        } else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) {
          hls.recoverMediaError();
        } else {
          notice('The player stopped: ' + data.details);
        }
      });
      hls.loadSource(url);
      hls.attachMedia(video);
    } else {
      // iPhone and iPad play HLS themselves
      video.src = url;
      q.appendChild(el('option', { text: 'Auto' }));
    }
    if (!$('chat').hidden) showChat(true);
  }

  $('quality').addEventListener('change', function () {
    if (!hls) return;
    hls.currentLevel = parseInt(this.value, 10);
    localStorage.setItem('quality', this.value === '-1' ? 'auto' : 'best');
  });

  function showChat(on) {
    var frame = $('chat');
    frame.hidden = !on;
    $('chat-toggle').className = on ? 'on' : '';
    localStorage.setItem('chat', on ? '1' : '0');
    var src = 'https://www.twitch.tv/embed/' + current + '/chat?darkpopout&parent=' + (listing.host || location.hostname);
    if (on && frame.getAttribute('src') !== src) frame.setAttribute('src', src);
    if (!on) frame.removeAttribute('src');
  }
  $('chat-toggle').addEventListener('click', function () { showChat($('chat').hidden); });

  $('follow').addEventListener('click', function () {
    var on = !listing.follows.some(function (f) { return f.login === current; });
    fetch('/api/follow', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ login: current, on: on }) }).then(loadLists);
  });

  $('copy').addEventListener('click', function () {
    var link = location.origin + '/live/' + current + '.m3u8', b = this;
    (navigator.clipboard ? navigator.clipboard.writeText(link) : Promise.reject()).then(function () {
      b.textContent = 'Copied';
    }, function () { window.prompt('Link for mpv, VLC or a TV', link); });
    setTimeout(function () { b.textContent = 'Copy link'; }, 1500);
  });

  function go(login, push) {
    if (!/^[A-Za-z0-9_]{2,25}$/.test(login)) return;
    if (push) history.pushState(null, '', '/watch/' + login.toLowerCase());
    watch(login);
    window.scrollTo(0, 0);
  }

  $('open').addEventListener('submit', function (e) {
    e.preventDefault();
    // a pasted twitch.tv address works too
    var name = $('name').value.trim().replace(/^https?:\/\/(www\.|m\.)?twitch\.tv\//, '').split(/[/?#]/)[0];
    go(name, true);
    $('name').value = '';
  });

  document.addEventListener('click', function (e) {
    var a = e.target.closest ? e.target.closest('a.card') : null;
    if (!a || e.ctrlKey || e.metaKey || e.shiftKey || e.button) return;
    e.preventDefault();
    go(a.getAttribute('href').split('/').pop(), true);
  });

  function route() {
    var m = location.pathname.match(/^\/watch\/([A-Za-z0-9_]{2,25})$/);
    if (m) watch(m[1]); else { stop(); current = null; $('watch').hidden = true; document.title = 'Twitch, no adverts'; }
  }
  window.addEventListener('popstate', route);

  function lamp() {
    fetch('/api/status').then(function (r) { return r.json(); }).then(function (s) {
      var x = s.exit, l = $('exit');
      if (!x.configured) { l.className = 'lamp warn'; l.textContent = 'home line: adverts cut, stream pauses'; }
      else if (!x.usable) { l.className = 'lamp bad'; l.textContent = 'exit down: adverts cut, stream pauses'; }
      else { l.className = 'lamp good'; l.textContent = x.country ? 'asking from ' + x.country : 'exit ready'; }
      l.title = x.last_error || '';
      var t = s.totals, cut = t.advert_segments_cut.exit + t.advert_segments_cut.home;
      $('totals').textContent = cut + ' advert segments cut, ' + t.stream_segments_passed + ' stream segments passed since start';
      var me = s.watching.filter(function (w) { return w.channel === current; })[0];
      if (me && me.advert_break) {
        var left = Math.max(0, Math.round((parseFloat(me.advert_break.seconds) || 0) - me.advert_break.for_seconds));
        notice('Twitch is running adverts on this stream. They are cut out; the picture returns' +
          (left ? ' in about ' + left + ' s.' : ' when they end.'));
      } else if ($('notice').textContent.indexOf('adverts') > 0) {
        notice('');
      }
    }).catch(function () { $('exit').className = 'lamp bad'; $('exit').textContent = 'service not answering'; });
  }

  if (localStorage.getItem('chat') === '1') { $('chat').hidden = false; $('chat-toggle').className = 'on'; }
  loadLists();
  route();
  lamp();
  setInterval(lamp, 3000);
  setInterval(loadLists, 60000);
})();
