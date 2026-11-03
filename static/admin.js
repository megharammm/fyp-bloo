/* Bloo AI — admin console. Vanilla JS, no framework. */
(function () {
  'use strict';

  const $ = (id) => document.getElementById(id);

  const state = {
    docId: null,
    chunkOffset: 0,
    chunkLimit: 10,
    chunkTotal: 0,
    chunks: []
  };

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  }[c]));

  async function api(path, opts) {
    const options = Object.assign({ headers: {} }, opts || {});
    if (options.body) options.headers['Content-Type'] = 'application/json';
    const res = await fetch(path, options);
    const text = await res.text();
    let body = null;
    try { body = text ? JSON.parse(text) : null; } catch (e) { body = { raw: text }; }
    if (!res.ok) throw new Error((body && body.error) || ('HTTP ' + res.status));
    return body;
  }

  function note(msg, isError) {
    const el = $('action-note');
    el.textContent = msg;
    el.style.color = isError ? '#ff8fa3' : '';
  }

  function badge(status) {
    return '<span class="badge b-' + esc(status) + '">' + esc(status) + '</span>';
  }

  function fmtDate(iso) {
    if (!iso) return '—';
    const d = new Date(iso);
    return isNaN(d) ? '—' : d.toLocaleString();
  }

  // ── stats ────────────────────────────────────────────────────────────
  async function loadStats() {
    try {
      const s = await api('/api/admin/stats');
      $('stat-documents').textContent = s.documents;
      $('stat-published').textContent = s.published;
      $('stat-chunks').textContent = s.chunks;
      $('stat-today').textContent = s.questions_today;
      $('stat-failed').textContent = s.failed;
    } catch (e) { /* leave dashes */ }
  }

  // ── documents table ──────────────────────────────────────────────────
  function query() {
    const params = new URLSearchParams();
    const q = $('q').value.trim();
    if (q) params.set('q', q);
    if ($('f-status').value) params.set('status', $('f-status').value);
    if ($('f-category').value) params.set('category', $('f-category').value);
    params.set('limit', '100');
    return params.toString();
  }

  async function loadDocs() {
    const body = await api('/api/admin/documents?' + query());
    $('total').textContent = body.total + ' document' + (body.total === 1 ? '' : 's');
    const tbody = $('docs');
    if (!body.documents.length) {
      tbody.innerHTML = '<tr><td colspan="6" class="muted">No documents match.</td></tr>';
      return;
    }
    tbody.innerHTML = body.documents.map((d) => {
      const err = d.error ? ' <span class="muted small" title="' + esc(d.error) + '">(error)</span>' : '';
      return '<tr data-id="' + esc(d.id) + '">' +
        '<td>' + esc(d.title) + err + '</td>' +
        '<td>' + esc(d.category) + '</td>' +
        '<td>' + badge(d.status) + '</td>' +
        '<td>' + (d.page_count == null ? '—' : d.page_count) + '</td>' +
        '<td>' + d.chunks + '</td>' +
        '<td class="muted small">' + fmtDate(d.updated_at) + '</td>' +
        '</tr>';
    }).join('');
  }

  // ── detail ───────────────────────────────────────────────────────────
  async function selectDoc(id) {
    state.docId = id;
    history.replaceState(null, '', '/admin?doc=' + id);
    $('detail').hidden = false;
    $('action-note').textContent = '';
    try {
      const body = await api('/api/admin/documents/' + id);
      const d = body.document;
      $('d-title').textContent = d.title;
      const st = $('d-status');
      st.className = 'badge b-' + d.status;
      st.textContent = d.status;
      $('d-meta').textContent = [
        d.kind, d.source_path || 'uploaded', d.version ? 'v' + d.version : '',
        d.chunks + ' chunks', d.page_count == null ? '' : d.page_count + ' pages'
      ].filter(Boolean).join(' · ');
      $('m-title').value = d.title;
      $('m-category').value = d.category;
      $('m-valid').value = d.valid_until || '';
      $('m-verified').value = d.last_verified_at || '';
      renderJobs(body.jobs);
      await loadChunks(0);
    } catch (e) {
      note('Cannot load document: ' + e.message, true);
    }
  }

  function renderJobs(jobs) {
    const el = $('d-jobs');
    if (!jobs || !jobs.length) {
      el.innerHTML = '<li class="muted">No jobs yet.</li>';
      return;
    }
    el.innerHTML = jobs.map((j) => {
      const progress = j.state === 'running' ? ' ' + j.progress + '%' : '';
      return '<li>' + j.kind + ' · ' + badge(j.state) + progress +
        ' · attempt ' + j.attempts +
        (j.error ? ' · <span style="color:#ff8fa3">' + esc(j.error.slice(0, 80)) + '</span>' : '') +
        '</li>';
    }).join('');
  }

  // ── chunks ───────────────────────────────────────────────────────────
  async function loadChunks(offset) {
    if (!state.docId) return;
    state.chunkOffset = Math.max(0, offset);
    const body = await api('/api/admin/documents/' + state.docId +
      '/chunks?offset=' + state.chunkOffset + '&limit=' + state.chunkLimit);
    state.chunks = body.chunks;
    state.chunkTotal = body.total;

    $('chunk-total').textContent = body.total + ' chunks';
    $('chunk-hint').textContent = body.total
      ? 'Click a chunk to read it. Breadcrumbs show where it sits in the document.'
      : 'This document produced no chunks.';

    $('chunk-list').innerHTML = body.chunks.map((c, i) => {
      const page = c.page_from ? 'p.' + c.page_from : '';
      const bread = c.section_path ? c.section_path.split(' > ').slice(-2).join(' > ') : '—';
      return '<li data-index="' + i + '"><span class="c-ord">#' + c.ord + '</span>' +
        '<span class="c-main"><span class="c-bread">' + esc(bread) + '</span>' +
        '<span class="muted small">' + (c.token_count || '?') + ' tok · ' + page +
        ' · ' + esc(c.chunk_type) + '</span></span></li>';
    }).join('');

    const from = body.total === 0 ? 0 : state.chunkOffset + 1;
    const to = Math.min(state.chunkOffset + state.chunkLimit, body.total);
    $('chunk-page').textContent = from + '–' + to + ' of ' + body.total;
    $('chunk-prev').disabled = state.chunkOffset === 0;
    $('chunk-next').disabled = state.chunkOffset + state.chunkLimit >= body.total;

    if (body.chunks.length) selectChunk(0);
    else $('chunk-view').innerHTML = '<p class="muted">No chunks.</p>';
  }

  function selectChunk(index) {
    const c = state.chunks[index];
    if (!c) return;
    Array.prototype.forEach.call($('chunk-list').children, (li, i) => {
      li.classList.toggle('active', i === index);
    });
    const head = '<div class="chunk-head">' +
      '<strong>#' + c.ord + '</strong> · ' + esc(c.chunk_type) +
      (c.page_from ? ' · p.' + c.page_from : '') +
      ' · ' + (c.token_count || '?') + ' tokens</div>' +
      '<div class="chunk-bread">' + esc(c.section_path || '(no section)') + '</div>';
    $('chunk-view').innerHTML = head +
      '<pre class="chunk-text"></pre>';
    $('chunk-view').querySelector('.chunk-text').textContent = c.text;
  }

  // ── actions ──────────────────────────────────────────────────────────
  async function callAction(name, message) {
    if (!state.docId) return;
    if (message && !confirm(message)) return;
    try {
      const res = await api('/api/admin/documents/' + state.docId + '/' + name, { method: 'POST' });
      note(name + ' ok' + (res && res.job_id ? ' (job queued)' : ''));
      await refreshAll();
      await selectDoc(state.docId);
    } catch (e) {
      note(name + ' failed: ' + e.message, true);
    }
  }

  async function saveMeta(ev) {
    ev.preventDefault();
    if (!state.docId) return;
    const body = {
      title: $('m-title').value,
      category: $('m-category').value,
      valid_until: $('m-valid').value || null,
      last_verified_at: $('m-verified').value || null
    };
    try {
      await api('/api/admin/documents/' + state.docId, { method: 'PATCH', body: JSON.stringify(body) });
      note('metadata saved');
      await refreshAll();
      await selectDoc(state.docId);
    } catch (e) {
      note('save failed: ' + e.message, true);
    }
  }

  async function deleteDoc() {
    if (!state.docId) return;
    if (!confirm('Delete this document, its chunks and the stored file? This cannot be undone.')) return;
    try {
      const res = await api('/api/admin/documents/' + state.docId, { method: 'DELETE' });
      state.docId = null;
      $('detail').hidden = true;
      history.replaceState(null, '', '/admin');
      note('deleted, removed ' + res.files_removed + ' file(s)');
      await refreshAll();
    } catch (e) {
      note('delete failed: ' + e.message, true);
    }
  }

  // ── upload ───────────────────────────────────────────────────────────
  function uploadFile(file) {
    const row = document.createElement('div');
    row.className = 'upload-row';
    row.innerHTML = '<span class="u-name">' + esc(file.name) + '</span>' +
      '<span class="u-bar"><i></i></span><span class="u-pct">0%</span>';
    $('uploads').appendChild(row);

    const bar = row.querySelector('.u-bar i');
    const pct = row.querySelector('.u-pct');

    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/api/admin/documents');
    xhr.upload.onprogress = (e) => {
      if (!e.lengthComputable) return;
      const p = Math.round((e.loaded / e.total) * 100);
      bar.style.width = p + '%';
      pct.textContent = p + '%';
    };
    xhr.onload = () => {
      let body = null;
      try { body = JSON.parse(xhr.responseText); } catch (e) { body = {}; }
      if (xhr.status >= 200 && xhr.status < 300) {
        pct.textContent = body.uploaded && body.uploaded[0] && body.uploaded[0].duplicate
          ? 'duplicate' : 'queued';
        row.classList.add('ok');
      } else {
        pct.textContent = 'failed';
        row.classList.add('bad');
        row.title = (body && body.error) || ('HTTP ' + xhr.status);
      }
      pollUntilIdle();
      refreshAll();
    };
    xhr.onerror = () => {
      pct.textContent = 'failed';
      row.classList.add('bad');
    };
    const fd = new FormData();
    fd.append('file', file, file.name);
    xhr.send(fd);
  }

  function handleFiles(files) {
    Array.prototype.forEach.call(files, uploadFile);
  }

  let pollTimer = null;
  async function pollUntilIdle() {
    if (pollTimer) return;
    let attempts = 0;
    pollTimer = setInterval(async () => {
      attempts += 1;
      try {
        const [q, r] = await Promise.all([
          api('/api/admin/jobs?state=queued&limit=1'),
          api('/api/admin/jobs?state=running&limit=1')
        ]);
        const busy = q.jobs.length + r.jobs.length;
        await refreshAll();
        if (busy === 0 || attempts > 60) {
          clearInterval(pollTimer);
          pollTimer = null;
          if (state.docId) selectDoc(state.docId);
        }
      } catch (e) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    }, 1200);
  }

  // ── sync ─────────────────────────────────────────────────────────────
  async function sync() {
    const out = $('syncOut');
    out.classList.remove('hidden');
    out.textContent = 'syncing…';
    try {
      const r = await api('/api/admin/sync', { method: 'POST' });
      out.textContent = 'new ' + r.new + ' · changed ' + r.changed +
        ' · unchanged ' + r.unchanged + ' · removed ' + r.removed +
        (r.errors && r.errors.length ? '\nerrors:\n' + r.errors.join('\n') : '');
      pollUntilIdle();
      refreshAll();
    } catch (e) {
      out.textContent = 'sync failed: ' + e.message;
    }
  }

  // ── wiring ───────────────────────────────────────────────────────────
  async function refreshAll() {
    await Promise.all([loadStats(), loadDocs()]);
  }

  $('refresh').addEventListener('click', async () => {
    await refreshAll();
    if (state.docId) selectDoc(state.docId);
  });

  $('q').addEventListener('input', debounce(loadDocs, 300));
  $('f-status').addEventListener('change', loadDocs);
  $('f-category').addEventListener('change', loadDocs);

  $('docs').addEventListener('click', (e) => {
    const tr = e.target.closest('tr[data-id]');
    if (tr) selectDoc(tr.getAttribute('data-id'));
  });

  $('meta-form').addEventListener('submit', saveMeta);
  $('btn-publish').addEventListener('click', () => callAction('publish'));
  $('btn-unpublish').addEventListener('click', () => callAction('unpublish'));
  $('btn-reingest').addEventListener('click', () => callAction('reingest', 'Re-parse this document from the stored file?'));
  $('btn-archive').addEventListener('click', () => callAction('archive', 'Archive this document? It will stop being answerable.'));
  $('btn-delete').addEventListener('click', deleteDoc);

  $('chunk-list').addEventListener('click', (e) => {
    const li = e.target.closest('li[data-index]');
    if (li) selectChunk(Number(li.getAttribute('data-index')));
  });
  $('chunk-prev').addEventListener('click', () => loadChunks(state.chunkOffset - state.chunkLimit));
  $('chunk-next').addEventListener('click', () => loadChunks(state.chunkOffset + state.chunkLimit));

  const drop = $('drop');
  const fileInput = $('file');
  drop.addEventListener('click', () => fileInput.click());
  drop.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') fileInput.click(); });
  fileInput.addEventListener('change', () => handleFiles(fileInput.files));
  ['dragenter', 'dragover'].forEach((ev) => drop.addEventListener(ev, (e) => {
    e.preventDefault();
    drop.classList.add('over');
  }));
  ['dragleave', 'drop'].forEach((ev) => drop.addEventListener(ev, (e) => {
    e.preventDefault();
    drop.classList.remove('over');
  }));
  drop.addEventListener('drop', (e) => {
    if (e.dataTransfer && e.dataTransfer.files) handleFiles(e.dataTransfer.files);
  });

  $('syncBtn').addEventListener('click', sync);

  function debounce(fn, ms) {
    let t = null;
    return function () {
      clearTimeout(t);
      t = setTimeout(fn, ms);
    };
  }

  // deep link ?doc=<uuid>
  const params = new URLSearchParams(location.search);
  refreshAll().then(() => {
    const id = params.get('doc');
    if (id) selectDoc(id);
  });
  setInterval(loadStats, 15000);
})();
