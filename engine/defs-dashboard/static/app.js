/* DEFS Dashboard Frontend */

const $ = id => document.getElementById(id);
const fmt = n => n.toLocaleString();
const fmtBytes = b => {
  if (b > 1e9) return (b / 1e9).toFixed(2) + ' GB';
  if (b > 1e6) return (b / 1e6).toFixed(2) + ' MB';
  if (b > 1e3) return (b / 1e3).toFixed(2) + ' KB';
  return b + ' B';
};
const fmtDate = ns => {
  if (!ns) return '—';
  const d = new Date(ns / 1e6);
  return d.toLocaleString();
};

// ─── Navigation ──────────────────────────────────────────────────
document.querySelectorAll('nav a').forEach(link => {
  link.addEventListener('click', e => {
    e.preventDefault();
    const section = link.dataset.section;
    document.querySelectorAll('nav a').forEach(l => l.classList.remove('active'));
    document.querySelectorAll('.section').forEach(s => s.classList.remove('active'));
    link.classList.add('active');
    $(section).classList.add('active');
    if (section === 'graph' && !graphLoaded) loadGraph();
  });
});

// ─── Stats ───────────────────────────────────────────────────────
async function loadStats() {
  try {
    const res = await fetch('/api/stats');
    const data = await res.json();
    $('status').textContent = 'Connected';
    $('status').style.color = 'var(--success)';

    $('stat-particles').textContent = fmt(data.particle_count);
    $('stat-singularities').textContent = fmt(data.singularity_count);
    $('stat-size').textContent = fmtBytes(data.size_mb * 1024 * 1024);
    $('stat-used').textContent = fmtBytes(data.used_bytes);

    $('vol-label').textContent = data.label;
    $('vol-version').textContent = data.version;
    $('vol-free').textContent = fmtBytes(data.free_bytes);
    $('vol-features').innerHTML = data.features.map(f => `<span class="tag tag-blue">${f}</span>`).join(' ') || '<span class="tag">none</span>';

    renderBarChart('dim-chart', data.dimensions);
    renderBarChart('bond-chart', data.bond_kinds);
  } catch (e) {
    $('status').textContent = 'Error: ' + e.message;
    $('status').style.color = 'var(--danger)';
  }
}

function renderBarChart(id, data) {
  const el = $(id);
  if (!data || !data.length) { el.innerHTML = '<div class="empty">No data</div>'; return; }
  const max = Math.max(...data.map(d => d[1]));
  el.innerHTML = data.map(([label, val]) => {
    const pct = max ? (val / max * 100).toFixed(1) : 0;
    return `<div class="bar-row">
      <div class="bar-label" title="${label}">${label}</div>
      <div class="bar-track"><div class="bar-fill" style="width:${pct}%"></div></div>
      <div class="bar-value">${fmt(val)}</div>
    </div>`;
  }).join('');
}

// ─── Particles ───────────────────────────────────────────────────
async function loadParticles() {
  try {
    const res = await fetch('/api/particles');
    const data = await res.json();
    $('particle-count').textContent = `(${fmt(data.length)})`;
    renderParticleTable('particle-table', data);
  } catch (e) {
    $('particle-table').innerHTML = `<tr><td colspan="7" class="empty">Error: ${e.message}</td></tr>`;
  }
}

function renderParticleTable(tbodyId, particles) {
  const tbody = $(tbodyId);
  if (!particles.length) { tbody.innerHTML = '<tr><td colspan="7" class="empty">No particles found</td></tr>'; return; }
  tbody.innerHTML = particles.map(p => `<tr>
    <td class="mono" title="${p.id}">${p.id.slice(0, 16)}…</td>
    <td>${escapeHtml(p.name || '—')}</td>
    <td><span class="tag">${escapeHtml(p.content_type || '—')}</span></td>
    <td>${p.dimension_count}</td>
    <td>${p.bond_count}</td>
    <td>${p.incoming_count}</td>
    <td>${fmtDate(p.modified_at)}</td>
  </tr>`).join('');
}

// ─── Search ──────────────────────────────────────────────────────
async function doSearch() {
  const q = $('search-input').value.trim();
  const kind = $('search-kind').value;
  const dim = $('search-dim').value;
  if (!q) return;

  const tbody = $('search-results');
  tbody.innerHTML = '<tr><td colspan="6" class="loading">Searching...</td></tr>';

  try {
    const res = await fetch(`/api/search?q=${encodeURIComponent(q)}&kind=${kind}&dim=${dim}`);
    const data = await res.json();
    renderParticleTable('search-results', data);
  } catch (e) {
    tbody.innerHTML = `<tr><td colspan="6" class="empty">Error: ${e.message}</td></tr>`;
  }
}

$('search-input').addEventListener('keydown', e => { if (e.key === 'Enter') doSearch(); });

// ─── Graph (Force-directed Canvas) ───────────────────────────────
let graphLoaded = false;
let graphData = null;

async function loadGraph() {
  try {
    const res = await fetch('/api/graph');
    graphData = await res.json();
    initGraph();
    graphLoaded = true;
  } catch (e) {
    $('graph-container').innerHTML = `<div class="empty">Error: ${e.message}</div>`;
  }
}

function initGraph() {
  const canvas = $('graph-canvas');
  const ctx = canvas.getContext('2d');
  const container = $('graph-container');
  const tooltip = $('graph-tooltip');

  const dpr = window.devicePixelRatio || 1;
  function resize() {
    const w = container.clientWidth;
    const h = container.clientHeight;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.scale(dpr, dpr);
  }
  resize();
  window.addEventListener('resize', resize);

  const nodes = graphData.nodes.map(n => ({
    ...n,
    x: Math.random() * container.clientWidth,
    y: Math.random() * container.clientHeight,
    vx: 0, vy: 0,
    radius: n.group === 'directory' ? 8 : 5,
  }));

  const nodeMap = new Map(nodes.map(n => [n.id, n]));
  const links = graphData.links
    .map(l => ({ ...l, source: nodeMap.get(l.source), target: nodeMap.get(l.target) }))
    .filter(l => l.source && l.target);

  const groups = { directory: '#58a6ff', text: '#3fb950', image: '#d29922', file: '#8b949e' };

  // Physics params
  const repulse = 400;
  const spring = 0.03;
  const damping = 0.85;
  const centerForce = 0.0005;

  let dragging = null;
  let hoverNode = null;
  let zoom = 1;
  let panX = 0, panY = 0;
  let animating = true;

  function step() {
    const cx = container.clientWidth / 2;
    const cy = container.clientHeight / 2;

    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      // Center gravity
      a.vx += (cx - a.x) * centerForce;
      a.vy += (cy - a.y) * centerForce;

      // Repulsion
      for (let j = i + 1; j < nodes.length; j++) {
        const b = nodes[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const f = repulse / (dist * dist);
        const fx = (dx / dist) * f;
        const fy = (dy / dist) * f;
        a.vx += fx; a.vy += fy;
        b.vx -= fx; b.vy -= fy;
      }
    }

    // Spring forces
    for (const link of links) {
      const dx = link.target.x - link.source.x;
      const dy = link.target.y - link.source.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const f = (dist - 80) * spring * link.strength;
      const fx = (dx / dist) * f;
      const fy = (dy / dist) * f;
      link.source.vx += fx; link.source.vy += fy;
      link.target.vx -= fx; link.target.vy -= fy;
    }

    // Integrate
    for (const n of nodes) {
      if (n === dragging) continue;
      n.vx *= damping;
      n.vy *= damping;
      n.x += n.vx;
      n.y += n.vy;
      // Bounds
      n.x = Math.max(20, Math.min(container.clientWidth - 20, n.x));
      n.y = Math.max(20, Math.min(container.clientHeight - 20, n.y));
    }
  }

  function draw() {
    const w = container.clientWidth;
    const h = container.clientHeight;
    ctx.clearRect(0, 0, w, h);

    ctx.save();
    ctx.translate(panX + w/2, panY + h/2);
    ctx.scale(zoom, zoom);
    ctx.translate(-w/2, -h/2);

    // Links
    for (const link of links) {
      ctx.beginPath();
      ctx.moveTo(link.source.x, link.source.y);
      ctx.lineTo(link.target.x, link.target.y);
      ctx.strokeStyle = 'rgba(139,148,158,0.2)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // Nodes
    for (const n of nodes) {
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius, 0, Math.PI * 2);
      ctx.fillStyle = groups[n.group] || groups.file;
      ctx.fill();
      if (n === hoverNode) {
        ctx.strokeStyle = '#fff';
        ctx.lineWidth = 2;
        ctx.stroke();
      }
    }

    // Labels for hover
    if (hoverNode) {
      ctx.fillStyle = '#fff';
      ctx.font = '12px sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(hoverNode.name, hoverNode.x, hoverNode.y - hoverNode.radius - 4);
    }

    ctx.restore();
  }

  function loop() {
    if (animating) {
      for (let i = 0; i < 3; i++) step();
    }
    draw();
    requestAnimationFrame(loop);
  }
  requestAnimationFrame(loop);

  // Interaction
  function getNodeAt(ex, ey) {
    const w = container.clientWidth, h = container.clientHeight;
    const x = (ex - (panX + w/2)) / zoom + w/2;
    const y = (ey - (panY + h/2)) / zoom + h/2;
    for (const n of nodes) {
      const dx = x - n.x, dy = y - n.y;
      if (dx*dx + dy*dy < (n.radius + 4)**2) return n;
    }
    return null;
  }

  canvas.addEventListener('mousedown', e => {
    const n = getNodeAt(e.offsetX, e.offsetY);
    if (n) { dragging = n; animating = false; }
  });

  canvas.addEventListener('mousemove', e => {
    if (dragging) {
      const w = container.clientWidth, h = container.clientHeight;
      dragging.x = (e.offsetX - (panX + w/2)) / zoom + w/2;
      dragging.y = (e.offsetY - (panY + h/2)) / zoom + h/2;
    } else {
      const n = getNodeAt(e.offsetX, e.offsetY);
      hoverNode = n;
      if (n) {
        tooltip.classList.add('visible');
        tooltip.style.left = (e.offsetX + 12) + 'px';
        tooltip.style.top = (e.offsetY + 12) + 'px';
        tooltip.innerHTML = `<strong>${escapeHtml(n.name)}</strong><br><span class="mono">${n.id.slice(0,20)}…</span><br>type: ${n.group}`;
      } else {
        tooltip.classList.remove('visible');
      }
    }
  });

  canvas.addEventListener('mouseup', () => { dragging = null; animating = true; });
  canvas.addEventListener('mouseleave', () => { dragging = null; hoverNode = null; tooltip.classList.remove('visible'); });

  canvas.addEventListener('wheel', e => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    zoom = Math.max(0.1, Math.min(5, zoom * factor));
  });
}

function escapeHtml(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// ─── Boot ──────────────────────────────────────────────────────────
loadStats();
loadParticles();
