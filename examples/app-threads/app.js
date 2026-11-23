// Studio: a demo of app threads (design §2.22). Each project is a thread
// of the app's own: created with a brief, talked to, its replies shown.
// Which thread is which project is the app's own state (stitch.storage).

const $ = (id) => document.getElementById(id);
const LABELS = {
  working: 'working', approval: 'waiting for approval', idle: 'your turn',
  done: 'done', cancelled: 'stopped',
};

let projects = [];        // [{name, thread}]
let rows = {};            // thread id -> its row from stitch.threads.list()
let current = null;       // thread id shown
let seen = -1;            // the last transcript entry shown
let attached = null;      // a File to send with the next message

function error(e) { $('error').textContent = e ? (e.message || String(e)) : ''; }

function showGrant(e) {
  const box = $('grant');
  if (!e || !e.consent || e.consent === 'allowed') { box.style.display = 'none'; return; }
  const what = {
    ask: 'Studio asks to run agent threads of its own, with:',
    changed: 'Studio now asks for different permissions:',
    denied: 'You did not let Studio run threads. To allow it:',
    none: 'Studio declares no threads.',
  }[e.consent] || e.message;
  box.innerHTML = '';
  const p = document.createElement('div');
  p.textContent = what;
  box.appendChild(p);
  if (e.needs) {
    const ul = document.createElement('ul');
    e.needs.forEach((n) => { const li = document.createElement('li'); li.textContent = n; ul.appendChild(li); });
    box.appendChild(ul);
  }
  if (e.allow) {
    const c = document.createElement('code');
    c.textContent = e.allow;
    box.appendChild(c);
    const t = document.createElement('div');
    t.textContent = 'Run it on a computer of yours; this page carries on once you have.';
    box.appendChild(t);
  }
  box.style.display = 'block';
}

function badge(state) {
  const b = document.createElement('span');
  b.className = 'badge ' + (state || '');
  b.textContent = LABELS[state] || state || '…';
  return b;
}

function renderSide() {
  const side = $('side');
  side.querySelectorAll('.project').forEach((n) => n.remove());
  $('none').style.display = projects.length ? 'none' : 'block';
  projects.forEach((p) => {
    const row = rows[p.thread] || {};
    const d = document.createElement('div');
    d.className = 'project' + (p.thread === current ? ' on' : '');
    const b = document.createElement('b');
    b.textContent = p.name;
    d.appendChild(b);
    d.appendChild(badge(row.state));
    if (row.cost) {
      const c = document.createElement('span');
      c.className = 'muted';
      c.textContent = '  $' + row.cost.toFixed(2);
      d.appendChild(c);
    }
    d.onclick = () => open(p.thread);
    side.appendChild(d);
  });
}

function bubble(m) {
  const d = document.createElement('div');
  d.className = 'msg ' + m.role;
  if (m.role === 'tool') {
    d.textContent = '· ' + m.name;
  } else {
    d.textContent = m.text;
    (m.images || []).forEach((img) => {
      const i = document.createElement('img');
      i.src = img.url;
      i.alt = img.name || '';
      d.appendChild(i);
    });
  }
  return d;
}

async function readMessages() {
  if (!current) return;
  const thread = current;
  const r = await stitch.threads.messages(thread, { after: seen });
  if (thread !== current) return;
  const log = $('log');
  r.messages.forEach((m) => { log.appendChild(bubble(m)); seen = Math.max(seen, m.n); });
  if (r.messages.length) log.scrollTop = log.scrollHeight;
  rows[thread] = r.state;
  renderSide();
}

async function open(thread) {
  current = thread;
  seen = -1;
  $('log').innerHTML = '';
  renderSide();
  try { await readMessages(); } catch (e) { error(e); }
}

async function refresh() {
  try {
    const r = await stitch.threads.list();
    rows = {};
    r.threads.forEach((t) => { rows[t.id] = t; });
    $('spend').textContent = `$${(r.spent_today || 0).toFixed(2)} of $${r.daily_usd} today`;
    showGrant(null);
    if (!current && projects.length) { await open(projects[projects.length - 1].thread); return; }
    renderSide();
    await readMessages();
  } catch (e) {
    showGrant(e);
    if (!e.consent) error(e);
  }
}

async function newProject() {
  const name = prompt('Project name (a garment, a collection):');
  if (!name) return;
  const brief = prompt('Brief for its agent: what should it design?') || name;
  error();
  try {
    const t = await stitch.threads.create({
      goal: `You are the design assistant for the garment project "${name}" in a ` +
        `fashion studio. Brief: ${brief}\nAnswer briefly; ask what you need to know.`,
      images: attached ? [attached] : [],
    });
    attached = null;
    $('attach').textContent = 'Image';
    projects.push({ name, thread: t.id });
    await stitch.storage.set('projects', projects);
    await refresh();
    await open(t.id);
  } catch (e) {
    showGrant(e);
    error(e);
  }
}

async function say(ev) {
  ev.preventDefault();
  const text = $('text').value.trim();
  if (!current || !text) return;
  error();
  try {
    await stitch.threads.send(current, text, attached ? [attached] : []);
    $('text').value = '';
    attached = null;
    $('attach').textContent = 'Image';
    setTimeout(readMessages, 300);
  } catch (e) { error(e); }
}

async function stop() {
  if (!current) return;
  try { await stitch.threads.cancel(current); await refresh(); } catch (e) { error(e); }
}

function init() {
  $('new').onclick = newProject;
  $('say').onsubmit = say;
  $('stop').onclick = stop;
  $('attach').onclick = () => $('pick').click();
  $('pick').onchange = () => {
    attached = $('pick').files[0] || null;
    $('attach').textContent = attached ? '1 image' : 'Image';
  };
  stitch.storage.get('projects').then((p) => {
    projects = Array.isArray(p) ? p : [];
    refresh();
  });
  // Something moved (a reply, a state), or the user granted: read again.
  stitch.threads.on(() => refresh());
  stitch.storage.onChange((key, value) => {
    if (key === 'projects') { projects = Array.isArray(value) ? value : []; renderSide(); }
  });
}

init();
