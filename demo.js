(() => {
  "use strict";

  const payload = window.PIXEL_PIPELINE_DEMO;
  if (!payload || !Array.isArray(payload.snapshots) || payload.snapshots.length === 0) {
    document.body.innerHTML = `
      <main class="fatal-error">
        <h1>Demo data is unavailable</h1>
        <p>Run <code>pwsh -File examples/generated-validation/build-demo.ps1</code>, then reload this file.</p>
      </main>`;
    return;
  }

  const byId = (id) => document.getElementById(id);
  const elements = {
    snapshotSelect: byId("snapshotSelect"),
    referenceSelect: byId("referenceSelect"),
    buildLabel: byId("buildLabel"),
    buildHash: byId("buildHash"),
    buildState: byId("buildState"),
    statAssets: byId("statAssets"),
    statExact: byId("statExact"),
    statThroughput: byId("statThroughput"),
    statGameReady: byId("statGameReady"),
    statBatch: byId("statBatch"),
    assetSearch: byId("assetSearch"),
    statusFilter: byId("statusFilter"),
    categoryFilters: byId("categoryFilters"),
    assetGrid: byId("assetGrid"),
    emptyState: byId("emptyState"),
    resultCount: byId("resultCount"),
    assetEyebrow: byId("assetEyebrow"),
    assetTitle: byId("assetTitle"),
    assetStyle: byId("assetStyle"),
    assetPosition: byId("assetPosition"),
    previousAsset: byId("previousAsset"),
    nextAsset: byId("nextAsset"),
    leftStageSelect: byId("leftStageSelect"),
    rightStageSelect: byId("rightStageSelect"),
    zoomRange: byId("zoomRange"),
    zoomValue: byId("zoomValue"),
    backdropToggle: byId("backdropToggle"),
    compareFrame: byId("compareFrame"),
    leftImage: byId("leftImage"),
    rightImage: byId("rightImage"),
    leftLabel: byId("leftLabel"),
    rightLabel: byId("rightLabel"),
    splitRange: byId("splitRange"),
    stageRail: byId("stageRail"),
    stageDescription: byId("stageDescription"),
    parameterTitle: byId("parameterTitle"),
    parameterSummary: byId("parameterSummary"),
    parameterStageId: byId("parameterStageId"),
    parameterGroups: byId("parameterGroups"),
    sourcePrompt: byId("sourcePrompt"),
    copyParameters: byId("copyParameters"),
    parameterCopyStatus: byId("parameterCopyStatus"),
    metricList: byId("metricList"),
    gateStatus: byId("gateStatus"),
    reasonList: byId("reasonList"),
    versionList: byId("versionList"),
    copyLink: byId("copyLink"),
    copyStatus: byId("copyStatus")
  };

  const query = new URLSearchParams(window.location.search);
  const snapshotExists = (id) => payload.snapshots.some((snapshot) => snapshot.id === id);
  const initialSnapshot = snapshotExists(query.get("snapshot")) ? query.get("snapshot") : payload.latest_snapshot;
  const initialReference = snapshotExists(query.get("reference")) ? query.get("reference") : initialSnapshot;

  const state = {
    snapshotId: initialSnapshot,
    referenceId: initialReference,
    assetId: query.get("asset"),
    category: "all",
    status: "all",
    search: "",
    leftStage: query.get("left") || "source",
    rightStage: query.get("right") || "final",
    checkerboard: true,
    zoom: 100
  };

  const stageNotes = {
    source: "Original generated candidate before deterministic processing.",
    native: "Recovered logical source grid after color voting and pixel-size detection.",
    body: "Composed body before the external outline is applied.",
    "body-mask": "Binary foreground contract used by cleanup and validation.",
    "outline-mask": "External silhouette plus bounded, supported internal contours.",
    final: "Exact target canvas after single-pass palette, contour compilation, and QA.",
    preview: "Nearest-neighbor inspection render for human review."
  };

  let activeParameterRecord = null;

  function currentSnapshot() {
    return payload.snapshots.find((snapshot) => snapshot.id === state.snapshotId) || payload.snapshots[0];
  }

  function referenceSnapshot() {
    return payload.snapshots.find((snapshot) => snapshot.id === state.referenceId) || currentSnapshot();
  }

  function humanizeAsset(asset) {
    const prefix = `${asset.category}-`;
    return asset.id.replace(prefix, "").split("-").map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(" ");
  }

  function assetFrom(snapshot, assetId) {
    return snapshot.assets.find((asset) => asset.id === assetId);
  }

  function selectedAsset() {
    const snapshot = currentSnapshot();
    let asset = assetFrom(snapshot, state.assetId);
    if (!asset) {
      asset = snapshot.assets[0];
      state.assetId = asset.id;
    }
    return asset;
  }

  function stageFrom(asset, stageId) {
    return asset.stages.find((stage) => stage.id === stageId) || asset.stages[0];
  }

  function createElement(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  function fillSelect(select, options, value) {
    select.replaceChildren();
    options.forEach(({ value: optionValue, label }) => {
      const option = document.createElement("option");
      option.value = optionValue;
      option.textContent = label;
      select.append(option);
    });
    select.value = value;
  }

  function updateUrl() {
    const url = new URL(window.location.href);
    url.searchParams.set("snapshot", state.snapshotId);
    url.searchParams.set("reference", state.referenceId);
    url.searchParams.set("asset", state.assetId);
    url.searchParams.set("left", state.leftStage);
    url.searchParams.set("right", state.rightStage);
    window.history.replaceState(null, "", url);
  }

  function renderSnapshotControls() {
    const options = payload.snapshots.map((snapshot) => ({
      value: snapshot.id,
      label: `${snapshot.label}${snapshot.dirty ? " (modified)" : ""}`
    }));
    fillSelect(elements.snapshotSelect, options, state.snapshotId);
    fillSelect(elements.referenceSelect, options, state.referenceId);
  }

  function renderBuild() {
    const snapshot = currentSnapshot();
    const stats = snapshot.stats;
    elements.buildLabel.textContent = snapshot.label;
    elements.buildHash.textContent = snapshot.commit_sha;
    elements.buildState.textContent = snapshot.dirty ? "Workspace changes included" : "Clean commit snapshot";
    elements.statAssets.textContent = stats.assets;
    elements.statExact.textContent = `${stats.exact_dimensions}/${stats.assets}`;
    elements.statThroughput.textContent = Number(stats.assets_per_second).toFixed(2);
    elements.statGameReady.textContent = `${stats.game_contract_valid}/${stats.assets}`;
    elements.statBatch.textContent = Math.round(stats.batch_duration_ms).toLocaleString();
  }

  function renderCategoryFilters() {
    const snapshot = currentSnapshot();
    const categories = ["all", ...new Set(snapshot.assets.map((asset) => asset.category))];
    elements.categoryFilters.replaceChildren();
    categories.forEach((category) => {
      const count = category === "all" ? snapshot.assets.length : snapshot.assets.filter((asset) => asset.category === category).length;
      const button = createElement("button", "", `${category} ${count}`);
      button.type = "button";
      button.setAttribute("aria-pressed", String(state.category === category));
      button.addEventListener("click", () => {
        state.category = category;
        renderCategoryFilters();
        renderLibrary();
      });
      elements.categoryFilters.append(button);
    });
  }

  function filteredAssets() {
    const needle = state.search.trim().toLowerCase();
    return currentSnapshot().assets.filter((asset) => {
      if (state.category !== "all" && asset.category !== state.category) return false;
      if (state.status !== "all" && asset.status !== state.status) return false;
      if (!needle) return true;
      return [asset.id, asset.category, asset.style, asset.profile, asset.target].join(" ").toLowerCase().includes(needle);
    });
  }

  function renderLibrary() {
    const assets = filteredAssets();
    elements.assetGrid.replaceChildren();
    elements.resultCount.textContent = `${assets.length} / ${currentSnapshot().assets.length}`;
    elements.emptyState.hidden = assets.length !== 0;

    assets.forEach((asset, index) => {
      const wrapper = createElement("div");
      wrapper.setAttribute("role", "listitem");
      const button = createElement("button", "asset-card");
      button.type = "button";
      button.style.setProperty("--index", index);
      button.setAttribute("aria-current", String(asset.id === state.assetId));
      button.setAttribute("aria-label", `Inspect ${humanizeAsset(asset)}`);

      const thumb = createElement("span", "asset-thumb");
      const preview = stageFrom(asset, "preview");
      const image = document.createElement("img");
      image.src = preview.path;
      image.alt = "";
      image.loading = "lazy";
      image.decoding = "async";
      thumb.append(image);

      const copy = createElement("span", "asset-card-copy");
      copy.append(createElement("strong", "", humanizeAsset(asset)));
      const meta = createElement("span", "asset-card-meta");
      meta.append(createElement("span", "", asset.target));
      meta.append(createElement("span", `asset-state ${asset.status}`, asset.status));
      copy.append(meta);
      button.append(thumb, copy);
      button.addEventListener("click", () => {
        state.assetId = asset.id;
        renderLibrary();
        renderAsset();
        elements.assetTitle.scrollIntoView({ block: "nearest", behavior: "smooth" });
      });
      wrapper.append(button);
      elements.assetGrid.append(wrapper);
    });
  }

  function setStageImage(image, stage, asset, versionLabel) {
    image.classList.add("is-loading");
    image.classList.toggle("pixelated", stage.id !== "source");
    image.classList.remove("is-missing");
    image.onload = () => image.classList.remove("is-loading");
    image.onerror = () => {
      image.classList.remove("is-loading");
      image.classList.add("is-missing");
    };
    image.src = stage.path;
    image.alt = `${humanizeAsset(asset)}, ${stage.label} processing stage from ${versionLabel}`;
  }

  function renderStageSelects(asset) {
    const options = asset.stages.map((stage) => ({ value: stage.id, label: stage.label }));
    if (!options.some((option) => option.value === state.leftStage)) state.leftStage = "source";
    if (!options.some((option) => option.value === state.rightStage)) state.rightStage = "final";
    fillSelect(elements.leftStageSelect, options, state.leftStage);
    fillSelect(elements.rightStageSelect, options, state.rightStage);
  }

  function renderComparison(asset) {
    const current = currentSnapshot();
    const reference = referenceSnapshot();
    const referenceAsset = assetFrom(reference, asset.id) || asset;
    const leftStage = stageFrom(referenceAsset, state.leftStage);
    const rightStage = stageFrom(asset, state.rightStage);

    setStageImage(elements.leftImage, leftStage, referenceAsset, reference.label);
    setStageImage(elements.rightImage, rightStage, asset, current.label);
    elements.leftLabel.textContent = `${reference.label} | ${leftStage.label}`;
    elements.rightLabel.textContent = `${current.label} | ${rightStage.label}`;
    elements.stageDescription.textContent = stageNotes[rightStage.id] || "Processing-stage evidence.";
    renderParameters(asset, rightStage);
    elements.compareFrame.style.setProperty("--zoom", String(state.zoom / 100));
    elements.zoomValue.textContent = `${state.zoom}%`;
  }

  function formatParameterValue(item) {
    const raw = item.value === null ? "auto" : String(item.value);
    return item.unit ? `${raw} ${item.unit}` : raw;
  }

  function renderParameters(asset, stage) {
    const groups = Array.isArray(stage.parameters) ? stage.parameters : [];
    const snapshot = currentSnapshot();
    elements.parameterTitle.textContent = `${stage.label} / recorded values`;
    elements.parameterSummary.textContent = stageNotes[stage.id] || "Recorded processing-stage values.";
    elements.parameterStageId.textContent = `${asset.id} :: ${stage.id}`;
    elements.parameterGroups.replaceChildren();

    if (groups.length === 0) {
      const empty = createElement("p", "parameter-empty", "This older snapshot does not contain stage parameter records.");
      elements.parameterGroups.append(empty);
    } else {
      groups.forEach((group) => {
        const section = createElement("section", "parameter-group");
        section.append(createElement("h4", "", group.name));
        const list = createElement("dl", "parameter-list");
        (group.items || []).forEach((item) => {
          const pair = createElement("div", "parameter-pair");
          pair.append(createElement("dt", "", item.name));
          const value = createElement("dd", "", formatParameterValue(item));
          if (item.source) value.append(createElement("small", "", item.source));
          pair.append(value);
          list.append(pair);
        });
        section.append(list);
        elements.parameterGroups.append(section);
      });
    }

    elements.sourcePrompt.textContent = asset.source.prompt || "Prompt was not recorded for this snapshot.";
    activeParameterRecord = {
      snapshot: snapshot.id,
      commit: snapshot.commit_sha,
      asset: asset.id,
      stage: stage.id,
      profile: asset.profile,
      profile_source: asset.process?.profile_source || null,
      effective_profile_sha256: asset.process?.effective_profile_sha256 || null,
      target_override: asset.process?.target_override || asset.target,
      parameters: groups,
      source_prompt: asset.source.prompt || null
    };
  }

  function renderStageRail(asset) {
    elements.stageRail.replaceChildren();
    asset.stages.forEach((stage) => {
      const button = createElement("button", "stage-button");
      button.type = "button";
      button.setAttribute("aria-pressed", String(stage.id === state.rightStage));
      button.setAttribute("aria-label", `Show ${stage.label} on the right`);
      const image = document.createElement("img");
      image.src = stage.path;
      image.alt = "";
      image.loading = "lazy";
      image.decoding = "async";
      button.append(image, createElement("span", "", stage.label));
      button.addEventListener("click", () => {
        state.rightStage = stage.id;
        elements.rightStageSelect.value = stage.id;
        renderComparison(asset);
        renderStageRail(asset);
        updateUrl();
      });
      elements.stageRail.append(button);
    });
  }

  function appendDefinitionList(list, entries, pairClass) {
    list.replaceChildren();
    entries.forEach(([term, value]) => {
      const pair = createElement("div", pairClass);
      pair.append(createElement("dt", "", term), createElement("dd", "", String(value)));
      list.append(pair);
    });
  }

  function renderEvidence(asset) {
    const contract = asset.game_contract || {
      id: "legacy-snapshot",
      runtime_role: "Unrecorded",
      anchor: "Unrecorded",
      source_valid: false,
      compile_valid: false,
      engine_filter: "Unrecorded",
      art_acceptance: "not-run",
      engine_acceptance: "not-run"
    };
    appendDefinitionList(elements.metricList, [
      ["Target", asset.target],
      ["Source", `${asset.source.width} × ${asset.source.height}`],
      ["Palette", `${asset.qa.palette_colors} / ${asset.qa.palette_limit}`],
      ["Body pixels", asset.qa.body_pixels.toLocaleString()],
      ["External outline", asset.qa.outline_pixels.toLocaleString()],
      ["Internal contours", (asset.qa.internal_outline_pixels ?? 0).toLocaleString()],
      ["Oklab error", asset.qa.perceptual_color_error_milli ?? "Unrecorded"],
      ["Strong-edge recall", asset.qa.detail_edge_recall_per_mille == null ? "Unrecorded" : `${asset.qa.detail_edge_recall_per_mille}‰`],
      ["Alpha", asset.qa.alpha_binary ? "Binary" : "Non-binary"],
      ["Components", asset.qa.body_components],
      ["Reserved border", asset.qa.reserved_border_pixels],
      ["Game contract", contract.id],
      ["Runtime role", contract.runtime_role],
      ["Anchor", contract.anchor],
      ["Source contract", contract.source_valid ? "Valid" : "Review"],
      ["Compile contract", contract.compile_valid ? "Valid" : "Review"],
      ["Art acceptance", contract.art_acceptance || "not-run"],
      ["Engine filter", contract.engine_filter],
      ["Engine import", contract.engine_acceptance]
    ], "metric-pair");

    elements.gateStatus.className = `gate-status ${asset.status}`;
    elements.gateStatus.textContent = asset.status;
    elements.reasonList.replaceChildren();
    const reasons = asset.reasons.length ? asset.reasons : ["No routing reasons"];
    reasons.forEach((reason) => elements.reasonList.append(createElement("li", "", reason)));

    const snapshot = currentSnapshot();
    appendDefinitionList(elements.versionList, [
      ["Tool", snapshot.tool_version],
      ["Commit", snapshot.commit_short],
      ["Branch", snapshot.branch || "detached"],
      ["Tree", snapshot.dirty ? "modified" : "clean"],
      ["Commit date", new Date(snapshot.commit_date).toLocaleDateString()],
      ["Snapshot", snapshot.id]
    ], "version-pair");
  }

  function renderAsset() {
    const snapshot = currentSnapshot();
    const asset = selectedAsset();
    const index = snapshot.assets.findIndex((entry) => entry.id === asset.id);
    elements.assetEyebrow.textContent = `${asset.category} / ${asset.profile} / ${asset.target}`;
    elements.assetTitle.textContent = humanizeAsset(asset);
    elements.assetStyle.textContent = asset.style;
    elements.assetPosition.textContent = `${index + 1} / ${snapshot.assets.length}`;
    renderStageSelects(asset);
    renderComparison(asset);
    renderStageRail(asset);
    renderEvidence(asset);
    updateUrl();
  }

  function navigateAsset(direction) {
    const assets = currentSnapshot().assets;
    const index = Math.max(0, assets.findIndex((asset) => asset.id === state.assetId));
    const nextIndex = (index + direction + assets.length) % assets.length;
    state.assetId = assets[nextIndex].id;
    renderLibrary();
    renderAsset();
  }

  function renderAll() {
    const snapshot = currentSnapshot();
    if (!assetFrom(snapshot, state.assetId)) state.assetId = snapshot.assets[0].id;
    if (!snapshotExists(state.referenceId)) state.referenceId = state.snapshotId;
    renderSnapshotControls();
    renderBuild();
    renderCategoryFilters();
    renderLibrary();
    renderAsset();
  }

  elements.snapshotSelect.addEventListener("change", (event) => {
    state.snapshotId = event.target.value;
    if (!assetFrom(currentSnapshot(), state.assetId)) state.assetId = currentSnapshot().assets[0].id;
    renderAll();
  });

  elements.referenceSelect.addEventListener("change", (event) => {
    state.referenceId = event.target.value;
    renderComparison(selectedAsset());
    updateUrl();
  });

  elements.assetSearch.addEventListener("input", (event) => {
    state.search = event.target.value;
    renderLibrary();
  });

  elements.statusFilter.addEventListener("change", (event) => {
    state.status = event.target.value;
    renderLibrary();
  });

  elements.leftStageSelect.addEventListener("change", (event) => {
    state.leftStage = event.target.value;
    renderComparison(selectedAsset());
    updateUrl();
  });

  elements.rightStageSelect.addEventListener("change", (event) => {
    state.rightStage = event.target.value;
    renderComparison(selectedAsset());
    renderStageRail(selectedAsset());
    updateUrl();
  });

  elements.splitRange.addEventListener("input", (event) => {
    elements.compareFrame.style.setProperty("--split", `${event.target.value}%`);
  });

  elements.zoomRange.addEventListener("input", (event) => {
    state.zoom = Number(event.target.value);
    elements.compareFrame.style.setProperty("--zoom", String(state.zoom / 100));
    elements.zoomValue.textContent = `${state.zoom}%`;
  });

  elements.backdropToggle.addEventListener("click", () => {
    state.checkerboard = !state.checkerboard;
    elements.compareFrame.classList.toggle("checkerboard", state.checkerboard);
    elements.backdropToggle.textContent = state.checkerboard ? "Grid / dark" : "Dark / grid";
  });

  elements.previousAsset.addEventListener("click", () => navigateAsset(-1));
  elements.nextAsset.addEventListener("click", () => navigateAsset(1));

  elements.copyLink.addEventListener("click", async () => {
    updateUrl();
    try {
      await navigator.clipboard.writeText(window.location.href);
      elements.copyStatus.textContent = "View URL copied.";
    } catch {
      elements.copyStatus.textContent = "Copy unavailable. Use the browser address bar.";
    }
    window.setTimeout(() => { elements.copyStatus.textContent = ""; }, 2400);
  });

  elements.copyParameters.addEventListener("click", async () => {
    if (!activeParameterRecord) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(activeParameterRecord, null, 2));
      elements.parameterCopyStatus.textContent = "Stage parameter JSON copied.";
    } catch {
      elements.parameterCopyStatus.textContent = "Copy unavailable. Parameter values remain visible below.";
    }
    window.setTimeout(() => { elements.parameterCopyStatus.textContent = ""; }, 2400);
  });

  renderAll();
})();
