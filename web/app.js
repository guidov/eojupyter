(() => {
  "use strict";

  const sidebarEl = document.getElementById("sidebar");
  const resizerEl = document.getElementById("resizer");
  const fileTreeEl = document.getElementById("file-tree");
  const filterInput = document.getElementById("filter-input");
  const rootForm = document.getElementById("root-form");
  const rootInput = document.getElementById("root-input");
  const btnParentDir = document.getElementById("btn-parent-dir");
  const btnToggleSidebar = document.getElementById("btn-toggle-sidebar");
  const btnTheme = document.getElementById("btn-theme");
  const btnModeIpynb = document.getElementById("btn-mode-ipynb");
  const btnModePy = document.getElementById("btn-mode-py");
  const btnModeAll = document.getElementById("btn-mode-all");
  const btnPrevCell = document.getElementById("btn-prev-cell");
  const btnNextCell = document.getElementById("btn-next-cell");
  const btnRefresh = document.getElementById("btn-refresh");
  const metaBadge = document.getElementById("meta-badge");
  const docHeader = document.getElementById("doc-header");
  const docPathEl = document.getElementById("doc-path");
  const docStatsEl = document.getElementById("doc-stats");
  const viewerScroll = document.getElementById("viewer-scroll");
  const notebookRoot = document.getElementById("notebook-root");
  const statusFile = document.getElementById("status-file");
  const statusScroll = document.getElementById("status-scroll");
  const imgModal = document.getElementById("img-modal");
  const imgModalImg = document.getElementById("img-modal-img");

  const state = {
    rootPath: "",
    filterMode: "ipynb",      // "ipynb" | "ipynb_py" | "all"
    filterQuery: "",
    dirCache: new Map(),      // dirPath -> TreeEntry[]
    expandedDirs: new Set(),  // dirPath
    searchResults: null,      // TreeEntry[] | null
    visibleNodes: [],         // flattened visible nodes
    selectedPath: null,       // currently highlighted tree row path
    activeFilePath: null,     // currently rendered file path
    docCache: new Map(),      // path -> rendered payload (LRU up to 20)
    renderAbort: null,
    activeCellIdx: -1,
  };

  const DOC_CACHE_MAX = 20;

  const CHEVRON_SVG = `<svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06Z"/></svg>`;
  const FOLDER_SVG = `<svg viewBox="0 0 16 16" width="15" height="15" fill="currentColor"><path d="M1.75 2.5A1.75 1.75 0 0 0 0 4.25v7.5C0 12.716.784 13.5 1.75 13.5h12.5A1.75 1.75 0 0 0 16 11.75v-5.5A1.75 1.75 0 0 0 14.25 4.5H7.5a.25.25 0 0 1-.2-.1l-.9-1.2A1.75 1.75 0 0 0 5 2.5H1.75Z"/></svg>`;
  const NB_SVG = `<svg viewBox="0 0 16 16" width="15" height="15" fill="currentColor"><path d="M2 2.75A1.75 1.75 0 0 1 3.75 1h8.5A1.75 1.75 0 0 1 14 2.75v10.5A1.75 1.75 0 0 1 12.25 15h-8.5A1.75 1.75 0 0 1 2 13.25V2.75Zm2.5 1.5a.75.75 0 0 0 0 1.5h7a.75.75 0 0 0 0-1.5h-7Zm0 3.25a.75.75 0 0 0 0 1.5h7a.75.75 0 0 0 0-1.5h-7Zm0 3.25a.75.75 0 0 0 0 1.5h4.5a.75.75 0 0 0 0-1.5h-4.5Z"/></svg>`;
  const FILE_SVG = `<svg viewBox="0 0 16 16" width="15" height="15" fill="currentColor"><path d="M2 1.75C2 .784 2.784 0 3.75 0h6.586c.464 0 .909.184 1.237.513l2.914 2.914c.329.328.513.773.513 1.237v9.586A1.75 1.75 0 0 1 13.25 16h-9.5A1.75 1.75 0 0 1 2 14.25Zm1.75-.25a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h9.5a.25.25 0 0 0 .25-.25V6h-2.75A1.75 1.75 0 0 1 9 4.25V1.5Zm6.75.062V4.25c0 .138.112.25.25.25h2.688l-.011-.013-2.914-2.914-.013-.011Z"/></svg>`;

  function formatBytes(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  }

  function cachePut(path, payload) {
    if (state.docCache.has(path)) {
      state.docCache.delete(path);
    }
    state.docCache.set(path, payload);
    if (state.docCache.size > DOC_CACHE_MAX) {
      const oldestKey = state.docCache.keys().next().value;
      state.docCache.delete(oldestKey);
    }
  }

  async function fetchDir(dirPath) {
    if (state.dirCache.has(dirPath)) {
      return state.dirCache.get(dirPath);
    }
    const qs = new URLSearchParams({
      path: dirPath,
      mode: state.filterMode,
    });
    const res = await fetch(`/api/list?${qs}`);
    if (!res.ok) return [];
    const entries = await res.json();
    state.dirCache.set(dirPath, entries);
    return entries;
  }

  function buildVisibleNodes() {
    const nodes = [];
    if (state.searchResults !== null) {
      for (const entry of state.searchResults) {
        nodes.push({ entry, depth: 0, parentPath: "" });
      }
      state.visibleNodes = nodes;
      return;
    }

    function walk(dirPath, depth) {
      const entries = state.dirCache.get(dirPath) || [];
      for (const entry of entries) {
        nodes.push({ entry, depth, parentPath: dirPath });
        if (entry.is_dir && state.expandedDirs.has(entry.path)) {
          walk(entry.path, depth + 1);
        }
      }
    }
    walk("", 0);
    state.visibleNodes = nodes;
  }

  function renderTree() {
    buildVisibleNodes();
    const frag = document.createDocumentFragment();

    if (state.visibleNodes.length === 0) {
      const empty = document.createElement("div");
      empty.className = "tree-empty";
      empty.textContent = state.filterQuery
        ? "No matching files."
        : state.filterMode === "ipynb"
          ? "No .ipynb notebooks in this folder. Switch to '+.py' or 'All'."
          : state.filterMode === "ipynb_py"
            ? "No .ipynb or .py files in this folder. Switch to 'All' to view other files."
            : "Empty directory.";
      frag.appendChild(empty);
      fileTreeEl.replaceChildren(frag);
      return;
    }

    for (const node of state.visibleNodes) {
      const { entry, depth } = node;
      const row = document.createElement("div");
      row.className = "tree-row";
      if (entry.path === state.selectedPath) {
        row.classList.add("selected");
      }
      row.dataset.path = entry.path;
      row.setAttribute("role", "treeitem");
      row.style.paddingLeft = `${8 + depth * 16}px`;

      const chev = document.createElement("span");
      chev.className = "tree-chevron";
      if (entry.is_dir) {
        chev.innerHTML = CHEVRON_SVG;
        if (state.expandedDirs.has(entry.path)) {
          chev.classList.add("expanded");
        }
      }
      row.appendChild(chev);

      const icon = document.createElement("span");
      const isPy = !entry.is_dir && entry.ext === "py";
      icon.className = `tree-icon ${
        entry.is_dir ? "is-dir" : entry.is_notebook ? "is-nb" : isPy ? "is-py" : "is-file"
      }`;
      icon.innerHTML = entry.is_dir ? FOLDER_SVG : entry.is_notebook ? NB_SVG : FILE_SVG;
      row.appendChild(icon);

      const label = document.createElement("span");
      label.className = "tree-label";
      label.textContent = entry.name;
      if (state.searchResults !== null && entry.path.includes("/")) {
        const sub = document.createElement("span");
        sub.className = "tree-subpath";
        sub.textContent = entry.path.slice(0, entry.path.lastIndexOf("/"));
        label.appendChild(sub);
      }
      row.appendChild(label);

      row.addEventListener("click", () => {
        selectNode(entry.path, true);
      });

      frag.appendChild(row);
    }

    fileTreeEl.replaceChildren(frag);
    scrollSelectedRowIntoView();
  }

  function scrollSelectedRowIntoView() {
    if (!state.selectedPath) return;
    const rows = fileTreeEl.children;
    for (let i = 0; i < rows.length; i++) {
      if (rows[i].dataset && rows[i].dataset.path === state.selectedPath) {
        rows[i].scrollIntoView({ block: "nearest" });
        break;
      }
    }
  }

  function updateSelectedHighlightOnly() {
    const rows = fileTreeEl.children;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i];
      if (!r.dataset) continue;
      if (r.dataset.path === state.selectedPath) {
        r.classList.add("selected");
        r.scrollIntoView({ block: "nearest" });
      } else {
        r.classList.remove("selected");
      }
    }
  }

  async function toggleDir(dirPath, forceExpand = null) {
    const isExpanded = state.expandedDirs.has(dirPath);
    const next = forceExpand === null ? !isExpanded : forceExpand;
    if (next) {
      state.expandedDirs.add(dirPath);
      await fetchDir(dirPath);
    } else {
      state.expandedDirs.delete(dirPath);
    }
    renderTree();
  }

  function prefetchNeighbors(currentPath) {
    const idx = state.visibleNodes.findIndex((n) => n.entry.path === currentPath);
    if (idx === -1) return;
    const candidates = [];
    for (const offset of [1, -1, 2]) {
      const neighbor = state.visibleNodes[idx + offset];
      if (
        neighbor &&
        !neighbor.entry.is_dir &&
        (neighbor.entry.is_notebook || neighbor.entry.ext === "py") &&
        !state.docCache.has(neighbor.entry.path)
      ) {
        candidates.push(neighbor.entry.path);
      }
    }
    if (candidates.length > 0) {
      fetch("/api/prefetch", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ paths: candidates }),
      }).catch(() => {});
    }
  }

  function applyRenderedPayload(payload) {
    state.activeFilePath = payload.path;
    state.activeCellIdx = -1;
    notebookRoot.innerHTML = payload.html;
    viewerScroll.scrollTop = 0;
    updateScrollIndicator();

    docHeader.hidden = false;
    docPathEl.textContent = payload.path;
    const cellWord = payload.kind === "notebook"
      ? `${payload.cell_count} cell${payload.cell_count === 1 ? "" : "s"}`
      : payload.kind === "text"
        ? `${payload.cell_count} lines`
        : payload.kind;
    docStatsEl.textContent = `${payload.kernel} · ${cellWord} · ${formatBytes(payload.file_size)} · ${payload.parse_ms} ms`;

    metaBadge.hidden = false;
    metaBadge.textContent = `${payload.kernel} · ${cellWord}`;
    statusFile.textContent = payload.path;
    document.title = `${payload.name} — eojupyter`;
  }

  async function openFile(path, forceReload = false) {
    if (!forceReload && state.docCache.has(path)) {
      applyRenderedPayload(state.docCache.get(path));
      prefetchNeighbors(path);
      return;
    }

    if (state.renderAbort) {
      state.renderAbort.abort();
    }
    const controller = new AbortController();
    state.renderAbort = controller;

    statusFile.textContent = `Loading ${path}…`;

    try {
      const qs = new URLSearchParams({ path });
      if (forceReload) qs.set("nocache", "1");
      const res = await fetch(`/api/render?${qs}`, { signal: controller.signal });
      if (!res.ok) {
        const err = await res.json().catch(() => ({ error: res.statusText }));
        notebookRoot.innerHTML = `<div class="nb-empty">Failed to render <code>${path}</code>: ${err.error || res.statusText}</div>`;
        return;
      }
      const payload = await res.json();
      if (state.selectedPath !== path && state.selectedPath !== null) {
        // User already moved away while awaiting
        cachePut(path, payload);
        return;
      }
      cachePut(path, payload);
      applyRenderedPayload(payload);
      prefetchNeighbors(path);
    } catch (err) {
      if (err.name !== "AbortError") {
        notebookRoot.innerHTML = `<div class="nb-empty">Error loading file: ${err.message}</div>`;
      }
    }
  }

  async function selectNode(path, toggleIfDir = false) {
    state.selectedPath = path;
    updateSelectedHighlightOnly();

    const node = state.visibleNodes.find((n) => n.entry.path === path);
    if (!node) return;

    if (node.entry.is_dir) {
      if (toggleIfDir) {
        await toggleDir(node.entry.path);
      }
    } else {
      await openFile(node.entry.path, false);
    }
  }

  function updateScrollIndicator() {
    const maxScroll = viewerScroll.scrollHeight - viewerScroll.clientHeight;
    if (maxScroll <= 0) {
      statusScroll.textContent = "100%";
      return;
    }
    const pct = Math.min(100, Math.max(0, Math.round((viewerScroll.scrollTop / maxScroll) * 100)));
    statusScroll.textContent = `${pct}%`;
  }

  function jumpCell(delta) {
    const cells = notebookRoot.querySelectorAll(".nb-cell");
    if (cells.length === 0) return;

    const currentTop = viewerScroll.scrollTop + 16;
    let currentIdx = -1;
    for (let i = 0; i < cells.length; i++) {
      if (cells[i].offsetTop <= currentTop) {
        currentIdx = i;
      } else {
        break;
      }
    }

    let targetIdx = currentIdx + delta;
    if (delta > 0 && state.activeCellIdx >= currentIdx && state.activeCellIdx < cells.length - 1) {
      targetIdx = state.activeCellIdx + 1;
    }
    targetIdx = Math.max(0, Math.min(cells.length - 1, targetIdx));
    state.activeCellIdx = targetIdx;

    for (const c of cells) c.classList.remove("cell-focused");
    const target = cells[targetIdx];
    target.classList.add("cell-focused");
    viewerScroll.scrollTo({
      top: Math.max(0, target.offsetTop - 12),
      behavior: "auto",
    });
  }

  async function runFilter(query) {
    state.filterQuery = query.trim();
    if (!state.filterQuery) {
      state.searchResults = null;
      renderTree();
      return;
    }
    const qs = new URLSearchParams({
      q: state.filterQuery,
      mode: state.filterMode,
    });
    const res = await fetch(`/api/search?${qs}`);
    if (!res.ok) return;
    state.searchResults = await res.json();
    if (state.searchResults.length > 0) {
      state.selectedPath = state.searchResults[0].path;
    }
    renderTree();
  }

  async function loadWorkspace(initialFile = null) {
    state.dirCache.clear();
    state.expandedDirs.clear();
    state.docCache.clear();
    state.searchResults = null;
    state.filterQuery = "";
    filterInput.value = "";

    await fetchDir("");
    renderTree();

    if (initialFile) {
      // Expand parent dirs if needed
      const parts = initialFile.split("/");
      let acc = "";
      for (let i = 0; i < parts.length - 1; i++) {
        acc = acc ? `${acc}/${parts[i]}` : parts[i];
        state.expandedDirs.add(acc);
        await fetchDir(acc);
      }
      renderTree();
      await selectNode(initialFile, false);
      return;
    }

    // Auto-select first notebook (or first file) in root
    const firstFile =
      state.visibleNodes.find((n) => !n.entry.is_dir && n.entry.is_notebook) ||
      state.visibleNodes.find((n) => !n.entry.is_dir);
    if (firstFile) {
      await selectNode(firstFile.entry.path, false);
    } else {
      docHeader.hidden = true;
      metaBadge.hidden = true;
      notebookRoot.innerHTML = `<div class="welcome-box"><h2>No notebooks in this folder</h2><p>Expand a subfolder with <strong>→</strong> or switch to <strong>All</strong> files.</p></div>`;
    }
  }

  async function switchRoot(newPath) {
    const res = await fetch("/api/root", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: newPath }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({ error: "Invalid path" }));
      statusFile.textContent = err.error;
      rootInput.value = state.rootPath;
      return;
    }
    const info = await res.json();
    state.rootPath = info.root;
    rootInput.value = info.root;
    await loadWorkspace(null);
  }

  // Event bindings
  viewerScroll.addEventListener("scroll", updateScrollIndicator, { passive: true });

  notebookRoot.addEventListener("click", (e) => {
    const copyBtn = e.target.closest(".nb-copy-btn");
    if (copyBtn) {
      const pre = copyBtn.parentElement.querySelector("pre");
      if (pre) {
        navigator.clipboard.writeText(pre.textContent || "").then(() => {
          copyBtn.textContent = "Copied";
          setTimeout(() => {
            copyBtn.textContent = "Copy";
          }, 1200);
        });
      }
      return;
    }

    const img = e.target.closest(".nb-img, .file-preview-image img");
    if (img && img.src) {
      imgModalImg.src = img.src;
      imgModal.hidden = false;
    }
  });

  imgModal.addEventListener("click", () => {
    imgModal.hidden = true;
    imgModalImg.src = "";
  });

  function applyTheme(theme) {
    const t = theme === "dark" ? "dark" : "light";
    document.documentElement.dataset.theme = t;
    btnTheme.textContent = t === "dark" ? "☀ Light" : "☾ Dark";
    try {
      localStorage.setItem("eojupyter-theme", t);
    } catch (_) {}
  }

  function toggleTheme() {
    const cur = document.documentElement.dataset.theme === "dark" ? "dark" : "light";
    applyTheme(cur === "dark" ? "light" : "dark");
  }

  try {
    applyTheme(localStorage.getItem("eojupyter-theme") || "light");
  } catch (_) {
    applyTheme("light");
  }

  btnTheme.addEventListener("click", toggleTheme);

  btnToggleSidebar.addEventListener("click", () => {
    sidebarEl.classList.toggle("collapsed");
  });

  btnPrevCell.addEventListener("click", () => jumpCell(-1));
  btnNextCell.addEventListener("click", () => jumpCell(1));

  btnRefresh.addEventListener("click", async () => {
    state.dirCache.clear();
    await fetchDir("");
    for (const d of state.expandedDirs) {
      await fetchDir(d);
    }
    renderTree();
    if (state.activeFilePath) {
      await openFile(state.activeFilePath, true);
    }
  });

  function updateModePills() {
    btnModeIpynb.classList.toggle("active", state.filterMode === "ipynb");
    btnModePy.classList.toggle("active", state.filterMode === "ipynb_py");
    btnModeAll.classList.toggle("active", state.filterMode === "all");
  }

  async function setFilterMode(mode) {
    if (state.filterMode === mode) return;
    state.filterMode = mode;
    updateModePills();
    state.dirCache.clear();
    await fetchDir("");
    for (const d of state.expandedDirs) await fetchDir(d);
    if (state.filterQuery) {
      await runFilter(state.filterQuery);
    } else {
      renderTree();
    }
  }

  btnModeIpynb.addEventListener("click", () => setFilterMode("ipynb"));
  btnModePy.addEventListener("click", () => setFilterMode("ipynb_py"));
  btnModeAll.addEventListener("click", () => setFilterMode("all"));

  rootForm.addEventListener("submit", (e) => {
    e.preventDefault();
    rootInput.blur();
    if (rootInput.value.trim() && rootInput.value.trim() !== state.rootPath) {
      switchRoot(rootInput.value.trim());
    }
  });

  btnParentDir.addEventListener("click", () => {
    if (!state.rootPath || state.rootPath === "/") return;
    const trimmed = state.rootPath.replace(/\/+$/, "");
    const idx = trimmed.lastIndexOf("/");
    const parent = idx <= 0 ? "/" : trimmed.slice(0, idx);
    switchRoot(parent);
  });

  let filterTimer = null;
  filterInput.addEventListener("input", () => {
    clearTimeout(filterTimer);
    filterTimer = setTimeout(() => runFilter(filterInput.value), 90);
  });

  // Resizable sidebar
  let isResizing = false;
  resizerEl.addEventListener("mousedown", (e) => {
    isResizing = true;
    resizerEl.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener("mousemove", (e) => {
    if (!isResizing) return;
    const width = Math.min(600, Math.max(180, e.clientX));
    sidebarEl.style.width = `${width}px`;
  });
  window.addEventListener("mouseup", () => {
    if (!isResizing) return;
    isResizing = false;
    resizerEl.classList.remove("dragging");
  });

  // Global keyboard navigation:
  // Arrow keys always navigate the left file explorer,
  // PgUp / PgDn / Space always scroll the right notebook pane smoothly without needing to change focus!
  window.addEventListener("keydown", async (e) => {
    if (!imgModal.hidden && e.key === "Escape") {
      imgModal.hidden = true;
      imgModalImg.src = "";
      e.preventDefault();
      return;
    }

    const active = document.activeElement;
    const inRootInput = active === rootInput;
    const inFilterInput = active === filterInput;

    if (inRootInput) {
      if (e.key === "Escape") {
        rootInput.value = state.rootPath;
        rootInput.blur();
      }
      return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
      e.preventDefault();
      sidebarEl.classList.toggle("collapsed");
      return;
    }

    if ((e.ctrlKey || e.metaKey) && (e.key.toLowerCase() === "p" || e.key.toLowerCase() === "f")) {
      e.preventDefault();
      if (sidebarEl.classList.contains("collapsed")) {
        sidebarEl.classList.remove("collapsed");
      }
      filterInput.focus();
      filterInput.select();
      return;
    }

    if (inFilterInput) {
      if (e.key === "Escape") {
        e.preventDefault();
        filterInput.value = "";
        filterInput.blur();
        await runFilter("");
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        filterInput.blur();
        if (state.selectedPath) {
          await selectNode(state.selectedPath, true);
        }
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") {
        return;
      }
    }

    // Notebook scroll keys (never hijacked by file tree focus!)
    if (e.key === "PageDown") {
      e.preventDefault();
      viewerScroll.scrollBy({ top: Math.floor(viewerScroll.clientHeight * 0.88), behavior: "auto" });
      return;
    }
    if (e.key === "PageUp") {
      e.preventDefault();
      viewerScroll.scrollBy({ top: -Math.floor(viewerScroll.clientHeight * 0.88), behavior: "auto" });
      return;
    }
    if (e.key === " " && !inFilterInput) {
      e.preventDefault();
      const dir = e.shiftKey ? -1 : 1;
      viewerScroll.scrollBy({ top: dir * Math.floor(viewerScroll.clientHeight * 0.85), behavior: "auto" });
      return;
    }
    if (e.key === "Home" && !inFilterInput) {
      e.preventDefault();
      viewerScroll.scrollTop = 0;
      return;
    }
    if (e.key === "End" && !inFilterInput) {
      e.preventDefault();
      viewerScroll.scrollTop = viewerScroll.scrollHeight;
      return;
    }

    if (e.key === "/" && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      if (sidebarEl.classList.contains("collapsed")) {
        sidebarEl.classList.remove("collapsed");
      }
      filterInput.focus();
      return;
    }

    if (e.key === "]" || e.key === "n") {
      e.preventDefault();
      jumpCell(1);
      return;
    }
    if (e.key === "[" || e.key === "p") {
      e.preventDefault();
      jumpCell(-1);
      return;
    }

    if (e.key === "F5" || (e.key === "r" && !e.ctrlKey && !e.metaKey)) {
      e.preventDefault();
      btnRefresh.click();
      return;
    }

    if (e.key === "t" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      toggleTheme();
      return;
    }

    if (e.key === "f" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      const nextMode = state.filterMode === "ipynb"
        ? "ipynb_py"
        : state.filterMode === "ipynb_py"
          ? "all"
          : "ipynb";
      await setFilterMode(nextMode);
      return;
    }

    // File tree navigation
    const nodes = state.visibleNodes;
    if (nodes.length === 0) return;

    let idx = nodes.findIndex((n) => n.entry.path === state.selectedPath);

    if (e.key === "ArrowDown" || (!inFilterInput && e.key === "j")) {
      e.preventDefault();
      const nextIdx = idx < 0 ? 0 : Math.min(nodes.length - 1, idx + 1);
      await selectNode(nodes[nextIdx].entry.path, false);
      return;
    }

    if (e.key === "ArrowUp" || (!inFilterInput && e.key === "k")) {
      e.preventDefault();
      const prevIdx = idx <= 0 ? 0 : idx - 1;
      await selectNode(nodes[prevIdx].entry.path, false);
      return;
    }

    if (idx < 0) return;
    const current = nodes[idx];

    if (e.key === "ArrowRight" || e.key === "l") {
      e.preventDefault();
      if (current.entry.is_dir) {
        if (!state.expandedDirs.has(current.entry.path)) {
          await toggleDir(current.entry.path, true);
        } else {
          // Move to first child if present
          const nextNode = state.visibleNodes[idx + 1];
          if (nextNode && nextNode.depth > current.depth) {
            await selectNode(nextNode.entry.path, false);
          }
        }
      }
      return;
    }

    if (e.key === "ArrowLeft" || e.key === "h") {
      e.preventDefault();
      if (current.entry.is_dir && state.expandedDirs.has(current.entry.path)) {
        await toggleDir(current.entry.path, false);
      } else if (current.parentPath) {
        await selectNode(current.parentPath, false);
      }
      return;
    }

    if (e.key === "Enter") {
      e.preventDefault();
      if (current.entry.is_dir) {
        await toggleDir(current.entry.path);
      } else {
        await openFile(current.entry.path, false);
      }
    }
  });

  // Boot
  (async function init() {
    try {
      const res = await fetch("/api/info");
      const info = await res.json();
      state.rootPath = info.root;
      rootInput.value = info.root;
      if (info.initial_file) {
        const lower = info.initial_file.toLowerCase();
        if (lower.endsWith(".py")) {
          state.filterMode = "ipynb_py";
        } else if (!lower.endsWith(".ipynb")) {
          state.filterMode = "all";
        }
        updateModePills();
      }
      await loadWorkspace(info.initial_file || null);
    } catch (err) {
      notebookRoot.innerHTML = `<div class="nb-empty">Failed to initialize viewer: ${err.message}</div>`;
    }
  })();
})();
