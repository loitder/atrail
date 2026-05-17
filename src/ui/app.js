const state = {
  sessions: [],
  summary: null,
  timeline: [],
  selectedId: null,
  search: "",
  status: "all",
  errorsOnly: false,
};

const elements = {
  statusPill: document.querySelector("#statusPill"),
  refreshButton: document.querySelector("#refreshButton"),
  metricSessions: document.querySelector("#metricSessions"),
  metricTurns: document.querySelector("#metricTurns"),
  metricTokens: document.querySelector("#metricTokens"),
  metricCached: document.querySelector("#metricCached"),
  metricTools: document.querySelector("#metricTools"),
  metricFailedTools: document.querySelector("#metricFailedTools"),
  metricAvgTurn: document.querySelector("#metricAvgTurn"),
  metricErrors: document.querySelector("#metricErrors"),
  sessionCount: document.querySelector("#sessionCount"),
  searchInput: document.querySelector("#searchInput"),
  statusFilter: document.querySelector("#statusFilter"),
  errorsOnly: document.querySelector("#errorsOnly"),
  sessionRows: document.querySelector("#sessionRows"),
  detailTitle: document.querySelector("#detailTitle"),
  detailStatus: document.querySelector("#detailStatus"),
  detailProject: document.querySelector("#detailProject"),
  detailModel: document.querySelector("#detailModel"),
  detailStarted: document.querySelector("#detailStarted"),
  detailDuration: document.querySelector("#detailDuration"),
  tokenTotal: document.querySelector("#tokenTotal"),
  tokenBars: document.querySelector("#tokenBars"),
  timelineCount: document.querySelector("#timelineCount"),
  timelineList: document.querySelector("#timelineList"),
};

const numberFormat = new Intl.NumberFormat();
const dateFormat = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
});

async function fetchJson(path) {
  const response = await fetch(path, { headers: { Accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`${path} returned ${response.status}`);
  }
  return response.json();
}

async function loadAll() {
  setStatus("Checking", "neutral");
  try {
    const [status, summary, sessions] = await Promise.all([
      fetchJson("/api/admin/status"),
      fetchJson("/api/metrics/summary"),
      fetchJson("/api/sessions"),
    ]);
    state.summary = summary;
    state.sessions = Array.isArray(sessions) ? sessions : [];
    setStatus(status.status === "ok" ? "Online" : "Degraded", status.status === "ok" ? "online" : "offline");
    renderSummary();
    renderSessions();

    const hashId = decodeURIComponent(window.location.hash.replace(/^#session=/, ""));
    const nextId = state.sessions.some((session) => session.id === hashId)
      ? hashId
      : state.selectedId || state.sessions[0]?.id || null;
    if (nextId) {
      await selectSession(nextId);
    } else {
      renderEmptyDetail();
    }
  } catch (error) {
    setStatus("Offline", "offline");
    renderError(error.message);
  }
}

function setStatus(label, tone) {
  elements.statusPill.textContent = label;
  elements.statusPill.className = `status-pill ${tone}`;
}

function renderSummary() {
  const summary = state.summary || {};
  elements.metricSessions.textContent = formatNumber(summary.sessions);
  elements.metricTurns.textContent = `${formatNumber(summary.turns)} turns`;
  elements.metricTokens.textContent = formatNumber(summary.total_tokens);
  elements.metricCached.textContent = `${formatNumber(summary.cached_tokens)} cached`;
  elements.metricTools.textContent = formatNumber(summary.tool_calls);
  elements.metricFailedTools.textContent = `${formatNumber(summary.failed_tool_calls)} failed`;
  elements.metricAvgTurn.textContent = formatDuration(summary.avg_turn_duration_ms);
  elements.metricErrors.textContent = `${formatNumber(summary.errors)} errors`;
  renderTokenBars(summary);
}

function renderTokenBars(summary) {
  const total = Number(summary.total_tokens || 0);
  const rows = [
    ["Input", summary.input_tokens, "input"],
    ["Output", summary.output_tokens, "output"],
    ["Cached", summary.cached_tokens, "cached"],
    ["Reasoning", summary.reasoning_tokens, "reasoning"],
  ];

  elements.tokenTotal.textContent = `${formatNumber(total)} total`;
  elements.tokenBars.innerHTML = rows
    .map(([label, value, type]) => {
      const safeValue = Number(value || 0);
      const width = total > 0 ? Math.max(2, Math.round((safeValue / total) * 100)) : 0;
      return `
        <div class="token-row">
          <span>${escapeHtml(label)}</span>
          <div class="token-track">
            <div class="token-fill ${type}" style="width: ${width}%"></div>
          </div>
          <span>${formatNumber(safeValue)}</span>
        </div>
      `;
    })
    .join("");
}

function renderSessions() {
  const filtered = filteredSessions();
  elements.sessionCount.textContent = `${formatNumber(filtered.length)} of ${formatNumber(state.sessions.length)} loaded`;

  if (!filtered.length) {
    elements.sessionRows.innerHTML = `<tr><td colspan="7" class="empty-cell">No sessions match the current filters</td></tr>`;
    return;
  }

  elements.sessionRows.innerHTML = filtered
    .map((session) => `
      <tr data-session-id="${escapeAttr(session.id)}" class="${session.id === state.selectedId ? "selected" : ""}">
        <td data-label="Session">
          <div class="session-id">
            <strong>${escapeHtml(shortId(session.id))}</strong>
            <span>${escapeHtml(session.project_id || "No project")}</span>
          </div>
        </td>
        <td data-label="Model">${escapeHtml(session.model || "unknown")}</td>
        <td data-label="Started">${formatDate(session.started_at)}</td>
        <td data-label="Duration">${formatDuration(session.duration_ms)}</td>
        <td data-label="Tokens">${formatNumber(session.total_tokens)}</td>
        <td data-label="Tools">${formatNumber(session.tool_call_count)}</td>
        <td data-label="Status"><span class="status-chip ${statusTone(session.status)}">${escapeHtml(session.status || "unknown")}</span></td>
      </tr>
    `)
    .join("");
}

function filteredSessions() {
  const needle = state.search.trim().toLowerCase();
  return state.sessions.filter((session) => {
    if (state.status !== "all" && session.status !== state.status) {
      return false;
    }
    if (state.errorsOnly && Number(session.error_count || 0) === 0) {
      return false;
    }
    if (!needle) {
      return true;
    }
    return [session.id, session.model, session.project_id, session.status]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(needle));
  });
}

async function selectSession(id) {
  state.selectedId = id;
  window.location.hash = `session=${encodeURIComponent(id)}`;
  renderSessions();
  const session = state.sessions.find((candidate) => candidate.id === id);
  renderDetail(session, true);
  try {
    state.timeline = await fetchJson(`/api/sessions/${encodeURIComponent(id)}/timeline`);
    renderTimeline();
  } catch (error) {
    elements.timelineList.innerHTML = `<p class="empty-state">${escapeHtml(error.message)}</p>`;
  }
}

function renderDetail(session, loadingTimeline = false) {
  if (!session) {
    renderEmptyDetail();
    return;
  }

  elements.detailTitle.textContent = shortId(session.id);
  elements.detailStatus.textContent = session.status || "unknown";
  elements.detailStatus.className = `status-chip ${statusTone(session.status)}`;
  elements.detailProject.textContent = session.project_id || "No project";
  elements.detailModel.textContent = session.model || "unknown";
  elements.detailStarted.textContent = formatDate(session.started_at);
  elements.detailDuration.textContent = formatDuration(session.duration_ms);
  elements.timelineCount.textContent = loadingTimeline ? "Loading" : `${formatNumber(state.timeline.length)} events`;
  if (loadingTimeline) {
    elements.timelineList.innerHTML = `<p class="empty-state">Loading timeline</p>`;
  }
}

function renderEmptyDetail() {
  elements.detailTitle.textContent = "None selected";
  elements.detailStatus.textContent = "Idle";
  elements.detailStatus.className = "status-chip neutral";
  elements.detailProject.textContent = "--";
  elements.detailModel.textContent = "--";
  elements.detailStarted.textContent = "--";
  elements.detailDuration.textContent = "--";
  elements.timelineCount.textContent = "-- events";
  elements.timelineList.innerHTML = `<p class="empty-state">No session selected</p>`;
}

function renderTimeline() {
  const items = Array.isArray(state.timeline) ? state.timeline : [];
  elements.timelineCount.textContent = `${formatNumber(items.length)} events`;

  if (!items.length) {
    elements.timelineList.innerHTML = `<p class="empty-state">No timeline events</p>`;
    return;
  }

  elements.timelineList.innerHTML = items
    .map((item) => {
      const title = item.name || item.event_type || "event";
      const details = [
        formatDate(item.timestamp),
        item.turn_id ? `turn ${shortId(item.turn_id)}` : null,
        item.duration_ms ? formatDuration(item.duration_ms) : null,
        item.status || null,
      ].filter(Boolean).join(" / ");
      return `
        <article class="timeline-item">
          <div class="timeline-item-head">
            <div>
              <div class="timeline-type">${escapeHtml(title)}</div>
              <div class="timeline-meta">${escapeHtml(item.event_type || "unknown")} / ${escapeHtml(details || "no metadata")}</div>
            </div>
            <span class="status-chip ${statusTone(item.status)}">${escapeHtml(item.status || "event")}</span>
          </div>
          <details>
            <summary>Attributes</summary>
            <pre>${escapeHtml(JSON.stringify(item.attributes || {}, null, 2))}</pre>
          </details>
        </article>
      `;
    })
    .join("");
}

function renderError(message) {
  elements.sessionRows.innerHTML = `<tr><td colspan="7" class="empty-cell">${escapeHtml(message)}</td></tr>`;
  elements.timelineList.innerHTML = `<p class="empty-state">${escapeHtml(message)}</p>`;
}

function formatNumber(value) {
  return numberFormat.format(Number(value || 0));
}

function formatDate(value) {
  if (!value) {
    return "--";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return dateFormat.format(date);
}

function formatDuration(value) {
  if (value === null || value === undefined) {
    return "--";
  }
  const ms = Number(value);
  if (!Number.isFinite(ms)) {
    return "--";
  }
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  if (ms < 60000) {
    return `${(ms / 1000).toFixed(1)} s`;
  }
  if (ms < 3600000) {
    return `${(ms / 60000).toFixed(1)} min`;
  }
  return `${(ms / 3600000).toFixed(1)} hr`;
}

function shortId(id) {
  if (!id) {
    return "--";
  }
  if (id.length <= 18) {
    return id;
  }
  return `${id.slice(0, 10)}...${id.slice(-6)}`;
}

function statusTone(status) {
  if (status === "success" || status === "ok") {
    return "success";
  }
  if (status === "failed" || status === "error") {
    return "failed";
  }
  if (status === "running") {
    return "running";
  }
  return "neutral";
}

function escapeHtml(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function escapeAttr(value) {
  return escapeHtml(value);
}

elements.refreshButton.addEventListener("click", loadAll);
elements.searchInput.addEventListener("input", (event) => {
  state.search = event.target.value;
  renderSessions();
});
elements.statusFilter.addEventListener("change", (event) => {
  state.status = event.target.value;
  renderSessions();
});
elements.errorsOnly.addEventListener("change", (event) => {
  state.errorsOnly = event.target.checked;
  renderSessions();
});
elements.sessionRows.addEventListener("click", (event) => {
  const row = event.target.closest("tr[data-session-id]");
  if (row) {
    selectSession(row.dataset.sessionId);
  }
});
window.addEventListener("hashchange", () => {
  const id = decodeURIComponent(window.location.hash.replace(/^#session=/, ""));
  if (id && id !== state.selectedId) {
    selectSession(id);
  }
});

loadAll();
