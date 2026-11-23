// Stitch's bridge for apps (design §2.10): first in every app's entry
// page, on the phone and in a browser alike. It gives the page:
// - `stitch.storage` and `localStorage`: the thread's state, synced to the
//   user's devices; read at once (the page came with it), written back
//   shortly after, and changed live when another device or page writes;
// - `stitch.report(data)`: data for the agent of the app's thread;
// - `stitch.live`: the node's threads, jobs and approvals, read-only, and
//   their changes, for an app the user let read them (asked natively on
//   the phone; `stitch app allow-live` for a browser);
// - `stitch.files`: the app's data folder (list, read, write, remove),
//   design §2.23;
// - `stitch.threads`: threads of the app's own (create, list, send,
//   messages, cancel) within what the user granted its manifest
//   (`stitch-app.json`; `stitch app allow-threads`), design §2.22;
// - its first fatal error told to the agent, and shown on the page;
// - in the phone's web view: downloads and printing through the system.
(function () {
  'use strict';
  if (window.stitch) return;
  var start = window.__stitch || {};
  var cache = Object.assign({}, start.state || {}); // key -> JSON text
  var generation = start.generation || 0;
  var native =
    window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.stitch;

  function post(msg) {
    if (native) {
      try { native.postMessage(msg); } catch (e) {}
    }
  }

  // --- State ---------------------------------------------------------------

  var pending = {}; // key -> JSON text, or null removed
  var timer = null;

  function parse(text) {
    try { return JSON.parse(text); } catch (e) { return null; }
  }

  function flush() {
    timer = null;
    var set = {}, remove = [], any = false;
    Object.keys(pending).forEach(function (k) {
      any = true;
      if (pending[k] === null) remove.push(k);
      else set[k] = parse(pending[k]);
    });
    if (!any) return Promise.resolve();
    var sent = pending;
    pending = {};
    return fetch('/_stitch/state', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ set: set, remove: remove }),
    }).then(function (r) {
      if (!r.ok) return r.text().then(function (t) { console.error('stitch.storage: ' + t); });
    }, function () {
      // Not sent: kept for the next write, unless overwritten since.
      Object.keys(sent).forEach(function (k) {
        if (!(k in pending)) pending[k] = sent[k];
      });
      schedule(2000);
    });
  }

  function schedule(ms) {
    if (!timer) timer = setTimeout(flush, ms);
  }

  function write(key, text) {
    key = String(key);
    if (text === null) delete cache[key];
    else cache[key] = text;
    pending[key] = text;
    schedule(200);
  }

  // Before the page goes, what is pending goes.
  function leave() {
    if (!Object.keys(pending).length) return;
    if (timer) { clearTimeout(timer); timer = null; }
    flush();
  }
  window.addEventListener('pagehide', leave);
  document.addEventListener('visibilitychange', function () {
    if (document.visibilityState === 'hidden') leave();
  });

  function fire(key, oldText, newText) {
    var ev;
    var init = {
      key: key,
      oldValue: oldText === undefined ? null : asString(oldText),
      newValue: newText === undefined ? null : asString(newText),
      url: location.href,
    };
    try { ev = new StorageEvent('storage', init); } catch (e) {
      ev = new CustomEvent('storage', { detail: init });
    }
    window.dispatchEvent(ev);
    listeners.forEach(function (f) {
      try {
        f(key, newText === undefined ? undefined : parse(newText));
      } catch (e) { console.error(e); }
    });
  }

  // What other devices and pages wrote, as it comes; live state's
  // changes too.
  var listeners = [];
  function poll() {
    var url = '/_stitch/events?generation=' + generation + '&live=' + (live.generation || 0);
    if (threads.consent !== 'none') url += '&threads=' + (threads.generation || 0);
    fetch(url, { cache: 'no-store' })
      .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
      .then(function (d) {
        generation = d.generation;
        if (d.live) heard(d.live);
        if (d.threads) moved(d.threads);
        var now = d.state || {};
        var keys = {};
        Object.keys(cache).concat(Object.keys(now)).forEach(function (k) { keys[k] = 1; });
        Object.keys(keys).forEach(function (k) {
          if (k in pending) return; // ours wins until it is written
          if (cache[k] !== now[k]) {
            var old = cache[k];
            if (now[k] === undefined) delete cache[k];
            else cache[k] = now[k];
            fire(k, old, now[k]);
          }
        });
        setTimeout(poll, 0);
      }, function () { setTimeout(poll, 5000); });
  }
  if (document.readyState === 'complete') setTimeout(poll, 500);
  else window.addEventListener('load', function () { setTimeout(poll, 500); });

  // localStorage holds strings: a string value is itself, another value
  // its JSON.
  function asString(text) {
    var v = parse(text);
    return typeof v === 'string' ? v : text;
  }

  var storage = {
    get: function (key) {
      var t = cache[String(key)];
      return Promise.resolve(t === undefined ? null : parse(t));
    },
    set: function (key, value) {
      write(key, JSON.stringify(value === undefined ? null : value));
      return Promise.resolve();
    },
    remove: function (key) {
      write(key, null);
      return Promise.resolve();
    },
    clear: function () {
      Object.keys(cache).forEach(function (k) { write(k, null); });
      return Promise.resolve();
    },
    keys: function () { return Promise.resolve(Object.keys(cache)); },
    all: function () {
      var out = {};
      Object.keys(cache).forEach(function (k) { out[k] = parse(cache[k]); });
      return Promise.resolve(out);
    },
    // Written now, not shortly after.
    flush: function () {
      if (timer) { clearTimeout(timer); timer = null; }
      return flush();
    },
    onChange: function (f) { listeners.push(f); },
    ready: Promise.resolve(),
  };

  var local = {
    get length() { return Object.keys(cache).length; },
    key: function (i) {
      var k = Object.keys(cache)[i];
      return k === undefined ? null : k;
    },
    getItem: function (k) {
      var t = cache[String(k)];
      return t === undefined ? null : asString(t);
    },
    setItem: function (k, v) { write(k, JSON.stringify(String(v))); },
    removeItem: function (k) { write(k, null); },
    clear: function () { storage.clear(); },
  };
  // The tab's own sessionStorage where the page may use it: it lasts across
  // reloads, as pages count on (the web client's "reload once" guard; one
  // kept in memory had every tab reloading for ever). Else the page's only.
  var tab = null;
  try {
    tab = window.sessionStorage;
    tab.setItem('__stitch', '1');
    tab.removeItem('__stitch');
  } catch (e) { tab = null; }
  var session = {};
  var sessionStore = tab || {
    get length() { return Object.keys(session).length; },
    key: function (i) {
      var k = Object.keys(session)[i];
      return k === undefined ? null : k;
    },
    getItem: function (k) {
      return Object.prototype.hasOwnProperty.call(session, k) ? session[k] : null;
    },
    setItem: function (k, v) { session[String(k)] = String(v); },
    removeItem: function (k) { delete session[String(k)]; },
    clear: function () { session = {}; },
  };
  try {
    Object.defineProperty(window, 'localStorage', { value: local, configurable: true });
  } catch (e) {}
  try {
    Object.defineProperty(window, 'sessionStorage', { value: sessionStore, configurable: true });
  } catch (e) {}

  function report(data) {
    var text = typeof data === 'string' ? data : JSON.stringify(data);
    return fetch('/_stitch/report', { method: 'POST', body: text }).then(function (r) {
      if (!r.ok) return r.text().then(function (t) { throw new Error(t); });
      // Where it went: `{ok, said}`.
      return r.json().catch(function () { return { ok: true }; });
    });
  }

  // --- Live state ------------------------------------------------------------

  var live = start.live || { consent: 'ask' };
  var liveOn = {}; // kind -> [callback]
  var KINDS = ['threads', 'jobs', 'attention', 'schedules', 'events'];

  function tell(kind, d) {
    (liveOn[kind] || []).concat(liveOn['*'] || []).forEach(function (f) {
      try { f(d); } catch (e) { console.error(e); }
    });
  }

  // What the long poll says of live state: allowed since, or a kind moved.
  function heard(now) {
    var was = live.consent;
    if (now.consent === 'allowed' && was !== 'allowed') {
      live = now;
      KINDS.forEach(function (k) { tell(k, { kind: k, consent: 'allowed' }); });
      return;
    }
    if (now.consent !== 'allowed') { live = now; return; }
    KINDS.forEach(function (k) {
      if (now[k] !== live[k]) tell(k, { kind: k, generation: now[k] });
    });
    live = now;
  }

  // Asked once at a time, natively (the page shows nothing of it).
  var asking = null;
  function ask() {
    if (!asking) {
      asking = new Promise(function (resolve) {
        window.__stitchLive = function (allowed) {
          asking = null;
          live.consent = allowed ? 'allowed' : 'denied';
          resolve(!!allowed);
        };
        post({ live: 'ask' });
      });
    }
    return asking;
  }

  function query(o) {
    var q = [];
    Object.keys(o || {}).forEach(function (k) {
      var v = o[k];
      if (v === undefined || v === null || v === false) return;
      if (v instanceof Date) v = v.toISOString();
      else if (k === 'since' && typeof v === 'number') v = new Date(v).toISOString();
      q.push(encodeURIComponent(k) + '=' + encodeURIComponent(v === true ? '1' : String(v)));
    });
    return q.length ? '?' + q.join('&') : '';
  }

  function read(kind, opts, asked) {
    return fetch('/_stitch/live/' + kind + query(opts), { cache: 'no-store' })
      .then(function (r) {
        if (r.status === 403) {
          return r.json().then(function (d) {
            if (d.consent === 'ask' && native && !asked) {
              return ask().then(function (ok) {
                if (ok) return read(kind, opts, true);
                d.consent = 'denied';
                d.error = '"' + d.name + '" may not see your threads, jobs and approvals: ' +
                  'you denied it.';
                return refuse(d);
              });
            }
            return refuse(d);
          });
        }
        if (!r.ok) return r.text().then(function (t) { throw new Error(t); });
        live.consent = 'allowed';
        return r.json();
      });
  }

  function looseWrite(q) {
    return fetch('/_stitch/live/loose-ends?' + q, { method: 'POST', cache: 'no-store' })
      .then(function (r) {
        if (!r.ok) return r.text().then(function (t) { throw new Error(t); });
        return r.json();
      });
  }

  function refuse(d) {
    var e = new Error(d.error);
    e.consent = d.consent; // 'ask' or 'denied'
    e.allow = d.allow; // the command that allows it
    throw e;
  }

  var liveApi = {
    // {all, status, since, limit}: the threads (this app's thread
    // started, or with `all` every detached thread the node knows).
    threads: function (o) { return read('threads', o); },
    jobs: function (o) { return read('jobs', o); },
    attention: function () { return read('attention'); },
    // {all}: every node's schedules and self-wakes, the soonest due
    // first (with `all`, the ended ones too).
    schedules: function (o) { return read('schedules', o); },
    // The conversations that wait on the user (loose ends), the latest
    // first: unread ones, and what the quick model judged of them.
    looseEnds: function () { return read('loose-ends'); },
    // {topic, limit}: the events outside programs published to this node
    // (docs/events.md), the newest first, with each ask's state, and the
    // subscriptions; `topic` is a pattern (`ci/**`).
    events: function (o) { return read('events', o); },
    // The built-in dashboard's own: a pass now, and a dismissal (until the
    // thread moves again). Other apps are answered 405.
    checkLooseEnds: function () { return looseWrite('check'); },
    dismissLooseEnd: function (thread) {
      return looseWrite('dismiss=' + encodeURIComponent(thread));
    },
    // `kind`: threads, jobs, attention, schedules, events, or '*'; the
    // callback gets {kind, generation}.
    on: function (kind, f) {
      if (kind !== '*' && KINDS.indexOf(kind) < 0) {
        throw new Error('stitch.live.on: threads, jobs, attention, schedules, events or *');
      }
      (liveOn[kind] = liveOn[kind] || []).push(f);
    },
    get consent() { return live.consent; },
  };

  // --- The app's own threads (design §2.22) ---------------------------------

  var threads = start.threads || { consent: 'none' };
  var threadsOn = [];

  // What the long poll says of the app's threads: something moved, or the
  // user granted (or took back) what the app asks.
  function moved(now) {
    var changed = now.generation !== threads.generation || now.consent !== threads.consent;
    threads = now;
    if (!changed) return;
    threadsOn.forEach(function (f) {
      try { f({ consent: now.consent, generation: now.generation }); } catch (e) { console.error(e); }
    });
  }

  function call(method, path, body) {
    var init = { method: method, cache: 'no-store' };
    if (body !== undefined) {
      init.headers = { 'Content-Type': 'application/json' };
      init.body = JSON.stringify(body);
    }
    return fetch('/_stitch/threads' + path, init).then(function (r) {
      return r.text().then(function (t) {
        var d = null;
        try { d = JSON.parse(t); } catch (e) {}
        if (r.ok) return d;
        var e = new Error((d && d.error) || t || ('HTTP ' + r.status));
        e.status = r.status;
        if (d && d.consent) {
          e.consent = d.consent; // 'ask', 'denied', 'changed' or 'none'
          e.allow = d.allow; // the command that grants it
          e.needs = d.needs; // what it asks, a line each
          threads.consent = d.consent;
        }
        throw e;
      });
    });
  }

  function id(t) { return encodeURIComponent(typeof t === 'object' ? t.id : t); }

  // Images: File or Blob objects, data: URLs, or {data, name}; sent as
  // data: URLs (PNG, JPEG, WebP, GIF; at most 4, 5 MiB each).
  function images(list) {
    return Promise.all((list || []).map(function (img) {
      if (typeof img === 'string' || (img && img.data)) return img;
      return new Promise(function (resolve, reject) {
        var fr = new FileReader();
        fr.onload = function () { resolve({ data: fr.result, name: img.name }); };
        fr.onerror = function () { reject(fr.error); };
        fr.readAsDataURL(img);
      });
    }));
  }

  var threadsApi = {
    // Where the app stands: {consent: allowed|ask|denied|changed|none,
    // needs: [line…], allow: 'stitch app allow-threads …'}.
    grant: function () { return call('GET', '/grant'); },
    // {goal, place, model, budget_usd, budget_minutes, max_steps, images}:
    // {id, place, model}.
    create: function (o) {
      o = Object.assign({}, o || {});
      return images(o.images).then(function (imgs) {
        if (imgs.length) o.images = imgs; else delete o.images;
        return call('POST', '', o);
      });
    },
    // {threads: [{id, title, goal, state, outcome, place, cost, last_words,
    // …}], spent_today, daily_usd, concurrent, at_work}; state is working,
    // approval (waiting on the user), idle, done or cancelled.
    list: function () { return call('GET', ''); },
    get: function (t) { return call('GET', '/' + id(t)); },
    // A message to one of its threads: woven into its turn at work, else
    // a new turn.
    send: function (t, text, imgs) {
      return images(imgs).then(function (list) {
        var body = { text: String(text) };
        if (list.length) body.images = list;
        return call('POST', '/' + id(t) + '/send', body);
      });
    },
    // {messages: [{n, role: user|assistant|tool, text, images:[{url}],
    // name, at}], state}; `after`: the last `n` the page has.
    messages: function (t, o) {
      var after = o && o.after !== undefined ? '?after=' + Number(o.after) : '';
      return call('GET', '/' + id(t) + '/messages' + after);
    },
    cancel: function (t) { return call('POST', '/' + id(t) + '/cancel', {}); },
    // Called when any of its threads may have moved (or the grant
    // changed): read again what is shown.
    on: function (f) { threadsOn.push(f); },
    get consent() { return threads.consent; },
  };

  // --- The app's data folder (design §2.23) ----------------------------------
  //
  // Its files, apart from its code, scoped to that folder alone: paths are
  // relative ("projects/a/recipe.json"), without `..` or hidden parts.
  function filePath(p) {
    return '/_stitch/files/' + String(p).split('/').map(encodeURIComponent).join('/');
  }
  function fileCall(url, init) {
    return fetch(url, Object.assign({ cache: 'no-store' }, init || {})).then(function (r) {
      if (r.ok) return r;
      return r.text().then(function (t) {
        var d = null;
        try { d = JSON.parse(t); } catch (e) {}
        var e = new Error((d && d.error) || t || ('HTTP ' + r.status));
        e.status = r.status;
        throw e;
      });
    });
  }
  var filesApi = {
    // {files: [{path, size, modified}], used, count, quota}; `dir` lists
    // only what is under it.
    list: function (dir) {
      return fileCall('/_stitch/files' + (dir ? '?dir=' + encodeURIComponent(dir) : ''))
        .then(function (r) { return r.json(); });
    },
    // The address of a file, for loaders (Three.js's GLTFLoader, <img>).
    url: function (p) { return filePath(p); },
    // Its bytes (an ArrayBuffer), text, or JSON.
    read: function (p) { return fileCall(filePath(p)).then(function (r) { return r.arrayBuffer(); }); },
    text: function (p) { return fileCall(filePath(p)).then(function (r) { return r.text(); }); },
    json: function (p) { return fileCall(filePath(p)).then(function (r) { return r.json(); }); },
    // Writes a string, an ArrayBuffer, a Blob or (a plain object) its JSON.
    write: function (p, data) {
      var body = data;
      if (data && typeof data === 'object' && !(data instanceof ArrayBuffer) &&
          !(typeof Blob !== 'undefined' && data instanceof Blob) && !ArrayBuffer.isView(data)) {
        body = JSON.stringify(data, null, 2) + '\n';
      }
      return fileCall(filePath(p), { method: 'POST', body: body }).then(function (r) { return r.json(); });
    },
    remove: function (p) {
      return fileCall(filePath(p) + '?remove', { method: 'POST' }).then(function (r) { return r.json(); });
    },
    // A mark that changes when the folder's files do (poll it).
    mark: function () {
      return fileCall('/_stitch/files?mark').then(function (r) { return r.json(); })
        .then(function (d) { return d.mark; });
    },
  };

  window.stitch = {
    name: start.name,
    version: start.version,
    pinned: !!start.pinned,
    storage: storage,
    report: report,
    live: liveApi,
    threads: threadsApi,
    files: filesApi,
  };
  // What edge talk's embeds called it.
  window.embedAPI = window.stitch;

  // --- Errors ----------------------------------------------------------------

  var first = '', count = 0, banner = null;
  function fatal(msg) {
    count++;
    if (!first) {
      first = String(msg);
      fetch('/_stitch/error', { method: 'POST', body: first }).catch(function () {});
      post({ log: 'fatal', message: first });
    }
    var show = function () {
      if (!document.body) return;
      if (!banner) {
        banner = document.createElement('div');
        banner.style.cssText =
          'position:fixed;top:0;left:0;right:0;z-index:2147483647;' +
          'background:rgba(255,59,48,0.92);color:#fff;' +
          'font:11.5px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;' +
          'padding:6px 10px;white-space:pre-wrap;word-break:break-word;' +
          'pointer-events:none;border-radius:0 0 8px 8px;';
        document.body.appendChild(banner);
      }
      banner.textContent =
        '⚠️ ' + first + (count > 1 ? '  (+' + (count - 1) + ' more)' : '');
    };
    if (document.body) show();
    else document.addEventListener('DOMContentLoaded', show);
  }
  window.addEventListener('error', function (e) {
    var t = e.target;
    if (t && t !== window && t.tagName) {
      // A script, a stylesheet, an image that did not load.
      fatal('could not load <' + t.tagName.toLowerCase() + '> ' + (t.src || t.href || ''));
    } else {
      fatal(e.message + (e.filename ? ' at ' + e.filename.replace(location.origin, '') : '') +
        (e.lineno ? ':' + e.lineno : ''));
    }
  }, true);
  window.addEventListener('unhandledrejection', function (e) {
    var r = e.reason;
    fatal('unhandled rejection: ' + (r && r.message ? r.message : String(r)));
  });

  // --- Size in a frame -----------------------------------------------------
  // When the page is shown in an iframe (the web client's transcript), it
  // tells the parent its content height, on load and when it changes,
  // throttled. One fixed message; the parent checks its sender and clamps it.
  if (window.parent && window.parent !== window) {
    var lastH = 0, sizeTimer = null;
    var height = function () {
      var d = document.documentElement, b = document.body;
      return Math.ceil(Math.max(d ? d.scrollHeight : 0, b ? b.scrollHeight : 0));
    };
    var tell = function () {
      sizeTimer = null;
      var h = height();
      if (!h || h === lastH) return;
      lastH = h;
      try { window.parent.postMessage({ type: 'stitch:resize', height: h }, '*'); } catch (e) {}
    };
    var schedule = function () {
      if (!sizeTimer) sizeTimer = setTimeout(tell, 100);
    };
    window.addEventListener('load', tell);
    if (window.ResizeObserver) {
      var ro = new ResizeObserver(schedule);
      var watch = function () {
        ro.observe(document.documentElement);
        if (document.body) ro.observe(document.body);
      };
      if (document.body) watch();
      else document.addEventListener('DOMContentLoaded', watch);
    }
  }

  if (!native) return;

  // --- The phone's web view --------------------------------------------------

  // Console lines to the app's log.
  ['log', 'warn', 'error'].forEach(function (level) {
    var orig = console[level];
    console[level] = function () {
      orig.apply(console, arguments);
      var text = Array.prototype.map.call(arguments, function (a) {
        if (a instanceof Error) return a.message + '\n' + (a.stack || '');
        if (typeof a === 'object') { try { return JSON.stringify(a); } catch (e) {} }
        return String(a);
      }).join(' ');
      post({ log: level, message: text });
    };
  });

  // The page's height, for a card in the conversation.
  function height() {
    var h = Math.max(document.body ? document.body.scrollHeight : 0,
      document.documentElement.scrollHeight);
    post({ height: h });
  }
  window.addEventListener('load', function () {
    height();
    if (window.ResizeObserver) {
      var last = 0;
      new ResizeObserver(function () {
        var now = Date.now();
        if (now - last > 250) { last = now; height(); }
        else setTimeout(height, 300);
      }).observe(document.documentElement);
    }
  });

  // No print dialog in a web view: the system's.
  window.print = function () { post({ print: true }); };

  // Downloads (`<a download>`, a blob saved): a file to share or save.
  var blobs = {};
  var create = URL.createObjectURL, revoke = URL.revokeObjectURL;
  URL.createObjectURL = function (b) {
    var u = create.call(URL, b);
    if (b instanceof Blob) blobs[u] = b;
    return u;
  };
  URL.revokeObjectURL = function (u) {
    // Kept a moment: `a.click(); URL.revokeObjectURL(u)` is common.
    setTimeout(function () { delete blobs[u]; }, 10000);
    revoke.call(URL, u);
  };
  function send(blob, name) {
    var reader = new FileReader();
    reader.onload = function () {
      post({
        download: String(reader.result).split(',')[1] || '',
        name: name || 'download',
        type: blob.type || 'application/octet-stream',
      });
    };
    reader.readAsDataURL(blob);
  }
  function download(href, name) {
    if (blobs[href]) return send(blobs[href], name);
    fetch(href).then(function (r) { return r.blob(); }).then(function (b) { send(b, name); },
      function (e) { console.error('download failed: ' + e.message); });
  }
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest && e.target.closest('a[download]');
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    download(a.href, a.getAttribute('download') || 'download');
  }, true);
  var click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (this.hasAttribute('download')) {
      return download(this.href, this.getAttribute('download') || 'download');
    }
    return click.call(this);
  };
  // pdfmake saves through an anchor it never adds to the page, and prints
  // through a frame: both become a download.
  var pdfMake;
  try {
    Object.defineProperty(window, 'pdfMake', {
      configurable: true,
      get: function () { return pdfMake; },
      set: function (v) {
        pdfMake = v;
        if (!v || !v.createPdf || v.createPdf.__stitch) return;
        var make = v.createPdf;
        v.createPdf = function () {
          var doc = make.apply(v, arguments);
          var save = function (name) {
            doc.getBlob(function (b) {
              send(b, typeof name === 'string' && name ? name : 'document.pdf');
            });
          };
          doc.download = save;
          doc.open = save;
          doc.print = save;
          return doc;
        };
        v.createPdf.__stitch = true;
      },
    });
  } catch (e) {}
})();
