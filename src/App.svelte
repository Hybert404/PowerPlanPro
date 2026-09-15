<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { getVersion } from '@tauri-apps/api/app';
  import { onMount } from 'svelte';

  const appWindow = getCurrentWindow();

  let highDropOpen = false;
  let lowDropOpen  = false;
  let version = '';

  function selectHigh(guid: string) { config.highLoadPlanGuid = guid; highDropOpen = false; saveConfig(); }
  function selectLow(guid: string)  { config.lowLoadPlanGuid  = guid; lowDropOpen  = false; saveConfig(); }

  type UsageSnapshot = {
    cpuUsage: number;
    gpuUsage: number;
    timestampMs: number;
  };

  type PowerPlan = {
    guid: string;
    name: string;
    isActive: boolean;
  };

  type ConditionMode = 'and' | 'or';

  type RuleConfig = {
    enabled: boolean;
    conditionMode: ConditionMode;
    cpuThreshold: number;
    gpuThreshold: number;
    durationSeconds: number;
    cooldownSeconds: number;
    highLoadPlanGuid: string;
    lowLoadPlanGuid: string;
  };

  type EngineStatus = {
    activePlanGuid: string;
    avgCpu: number;
    avgGpu: number;
    conditionMet: boolean;
    targetPlanGuid: string;
    switchesCount: number;
    paused: boolean;
    resumeAtMs: number | null;
  };

  type PlanSegment = {
    guid: string;
    startIndex: number; // absolute sample index when this plan became active
  };

  // Always retain 600 samples (the max 10-min window); display is sliced per selection.
  const MAX_POINTS = 600;
  const CHART_WIDTH = 700;
  const CHART_HEIGHT = 140;

  const TIMEFRAMES = [
    { label: '1 min', points: 60 },
    { label: '5 min', points: 300 },
    { label: '10 min', points: 600 },
  ] as const;
  let viewPoints = 300; // default: 5 min

  // Chart band colors — keyed to plan category to match the background glow
  function planColor(guid: string): string {
    const cat = planCategory(guid);
    if (cat === 'balanced')    return 'rgba(40, 110, 255, 0.18)';
    if (cat === 'saver')       return 'rgba(0, 210, 100, 0.18)';
    if (cat === 'performance') return 'rgba(255, 50, 20, 0.18)';
    return 'rgba(190, 90, 255, 0.18)';
  }

  /** Map the active plan to a visual theme using well-known Windows GUIDs first,
   *  falling back to localisation-independent keyword matching on the name.
   *  Built-in GUIDs are stable across all Windows locales. */
  function planCategory(guid: string): 'saver' | 'performance' | 'balanced' {
    const g = guid.toLowerCase();
    // Power Saver
    if (g === 'a1841308-3541-4fab-bc81-f71556f20b4a') return 'saver';
    // High Performance
    if (g === '8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c') return 'performance';
    // Ultimate Performance
    if (g === 'e9a42b02-d5df-448d-aa00-03f14749eb61') return 'performance';
    // Balanced
    if (g === '381b4222-f694-41f0-9685-ff5bb260df2e') return 'balanced';
    // Unknown GUID — fall back to English + common localisation substrings
    const name = planName(guid).toLowerCase();
    if (
      name.includes('sav') || name.includes('eco') ||
      name.includes('oszcz') || name.includes('spars')
    ) return 'saver';
    if (
      name.includes('high') || name.includes('perf') ||
      name.includes('ultim') || name.includes('boost') ||
      name.includes('gaming') || name.includes('wysok') ||
      name.includes('leistung')
    ) return 'performance';
    return 'balanced';
  }

  let cpuUsage = 0;
  let gpuUsage = 0;
  let lastUpdated = '-';
  let errorText = '';

  let plans: PowerPlan[] = [];
  let status: EngineStatus | null = null;
  let autostartEnabled = false;
  let config: RuleConfig = {
    enabled: false,
    conditionMode: 'or',
    cpuThreshold: 70,
    gpuThreshold: 70,
    durationSeconds: 30,
    cooldownSeconds: 20,
    highLoadPlanGuid: '',
    lowLoadPlanGuid: ''
  };

  let pauseMinutes = 5;
  let pauseCountdown = '';

  let cpuHistory: number[] = [];
  let gpuHistory: number[] = [];
  let totalSamples = 0;
  let planSegments: PlanSegment[] = [];
  let lastKnownPlanGuid = '';

  let svgEl: SVGSVGElement;
  let chartGroupEl: SVGGElement;
  let scrollAnim: Animation | null = null;

  function triggerScrollAnimate() {
    if (!svgEl || !chartGroupEl) return;
    const { width } = svgEl.getBoundingClientRect();
    if (width === 0) return;
    const xStepPx = width / Math.max(viewPoints - 1, 1);
    scrollAnim?.finish();
    scrollAnim = chartGroupEl.animate(
      [
        { transform: `translateX(${xStepPx}px)` },
        { transform: 'translateX(0px)' }
      ],
      { duration: 400, easing: 'cubic-bezier(0.25, 0, 0.35, 1)' }
    );
  }

  function toPath(values: number[], vp: number) {
    // Render vp+1 points: x from -xStep (off left edge) to CHART_WIDTH.
    // The extra left point fills the gap when the group is shifted right
    // by one step at the start of each scroll animation.
    const xStep = CHART_WIDTH / Math.max(vp - 1, 1);
    const needed = vp + 1;
    const first = values[0] ?? 0;
    const padded = values.length >= needed
      ? values.slice(-needed)
      : Array(needed - values.length).fill(first).concat(values);
    const pts = padded.map((value, index) => ({
      x: (index - 1) * xStep,  // index 0 → x=-xStep, index vp → x=CHART_WIDTH
      y: CHART_HEIGHT - (Math.max(0, Math.min(100, value)) / 100) * CHART_HEIGHT,
    }));
    let d = `M ${pts[0].x.toFixed(2)} ${pts[0].y.toFixed(2)}`;
    for (let i = 0; i < pts.length - 1; i++) {
      const cp = xStep * 0.4;
      const x1 = (pts[i].x + cp).toFixed(2);
      const y1 = pts[i].y.toFixed(2);
      const x2 = (pts[i + 1].x - cp).toFixed(2);
      const y2 = pts[i + 1].y.toFixed(2);
      d += ` C ${x1} ${y1} ${x2} ${y2} ${pts[i + 1].x.toFixed(2)} ${pts[i + 1].y.toFixed(2)}`;
    }
    return d;
  }

  function toAreaPath(values: number[], vp: number) {
    const line = toPath(values, vp);
    if (!line) return '';
    const xStep = CHART_WIDTH / Math.max(vp - 1, 1);
    return `${line} L ${CHART_WIDTH} ${CHART_HEIGHT} L ${(-xStep).toFixed(2)} ${CHART_HEIGHT} Z`;
  }

  function pushValue(buffer: number[], value: number): number[] {
    const next = [...buffer, value];
    if (next.length > MAX_POINTS) next.shift();
    return next;
  }

  function trackPlanChange(guid: string) {
    if (!guid || guid === lastKnownPlanGuid) return;
    lastKnownPlanGuid = guid;
    let segs = [...planSegments, { guid, startIndex: totalSamples }];
    // Drop segments that have scrolled fully off the visible window
    while (segs.length > 1 && segs[1].startIndex <= totalSamples - MAX_POINTS) {
      segs = segs.slice(1);
    }
    planSegments = segs;
  }

  function computePlanBands(history: number[], total: number, segments: PlanSegment[], vp: number) {
    const n = history.length;
    if (n === 0 || segments.length === 0) return [];
    const oldest = total - n;
    // Use the same xStep and left-offset as toPath so bands align with lines.
    const xStep = CHART_WIDTH / Math.max(vp - 1, 1);
    const offset = vp - n;
    const result: { guid: string; x: number; width: number }[] = [];
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      const nextStart = i + 1 < segments.length ? segments[i + 1].startIndex : total;
      if (nextStart <= oldest) continue;
      const relFrom = Math.max(seg.startIndex - oldest, 0);
      const relTo = Math.min(nextStart - oldest, n - 1);
      const x1 = (offset + relFrom) * xStep;
      const x2 = Math.min((offset + relTo) * xStep, CHART_WIDTH);
      if (x2 > x1) result.push({ guid: seg.guid, x: x1, width: x2 - x1 });
    }
    // Extend first band one step to the left so it fills the gap during the
    // scroll animation when the group is shifted right by one xStep.
    if (result.length > 0) {
      const xStepExt = CHART_WIDTH / Math.max(vp - 1, 1);
      result[0] = { ...result[0], x: result[0].x - xStepExt, width: result[0].width + xStepExt };
    }
    return result;
  }

  $: activeCategory = status ? planCategory(status.activePlanGuid) : 'balanced';
  // Slice history to the chosen view window
  $: cpuView = cpuHistory.slice(-viewPoints);
  $: gpuView = gpuHistory.slice(-viewPoints);
  $: planBands = computePlanBands(cpuView, totalSamples, planSegments, viewPoints);
  $: seenPlanGuids = [...new Set(planSegments.map((s) => s.guid))];

  // Y coordinates for the right-side live-value labels (SVG user units)
  $: rawCpuY = CHART_HEIGHT - (Math.max(0, Math.min(100, cpuUsage)) / 100) * CHART_HEIGHT;
  $: rawGpuY = CHART_HEIGHT - (Math.max(0, Math.min(100, gpuUsage)) / 100) * CHART_HEIGHT;
  // Keep labels at least 14 units apart; CPU gets priority position
  $: cpuLabelY = rawCpuY;
  $: gpuLabelY = Math.abs(rawGpuY - rawCpuY) < 14 ? rawCpuY + 14 : rawGpuY;

  async function sampleUsage() {
    try {
      const snapshot = await invoke<UsageSnapshot>('get_usage_snapshot');
      cpuUsage = snapshot.cpuUsage;
      gpuUsage = snapshot.gpuUsage;
      cpuHistory = pushValue(cpuHistory, cpuUsage);
      gpuHistory = pushValue(gpuHistory, gpuUsage);
      totalSamples++;
      if (status) trackPlanChange(status.activePlanGuid);
      lastUpdated = new Date(snapshot.timestampMs).toLocaleTimeString();
      errorText = '';
      triggerScrollAnimate();
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function refreshEngineStatus() {
    const nextStatus = await invoke<EngineStatus>('get_engine_status');
    status = nextStatus;
  }

  async function loadInitialData() {
    plans = await invoke<PowerPlan[]>('list_power_plans');
    config = await invoke<RuleConfig>('get_rule_config');
    autostartEnabled = await invoke<boolean>('get_autostart');
    await refreshEngineStatus();
    if (status) trackPlanChange(status.activePlanGuid);
  }

  async function saveConfig() {
    try {
      await invoke('set_rule_config', { config });
      errorText = '';
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function applyAutostart() {
    try {
      await invoke('set_autostart', { enabled: autostartEnabled });
      errorText = '';
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  let switchingPlan = false;
  async function setPlan(planGuid: string) {
    if (switchingPlan) return;
    switchingPlan = true;
    try {
      await invoke('set_power_plan', { planGuid });
      await refreshEngineStatus();
      errorText = '';
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    } finally {
      setTimeout(() => { switchingPlan = false; }, 500);
    }
  }

  async function applyPause() {
    try {
      await invoke('pause_engine', { minutes: pauseMinutes });
      await refreshEngineStatus();
      errorText = '';
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function cancelPause() {
    try {
      await invoke('unpause_engine');
      await refreshEngineStatus();
      pauseCountdown = '';
      errorText = '';
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  function updateCountdown() {
    if (!status?.paused || !status.resumeAtMs) {
      pauseCountdown = '';
      return;
    }
    const remaining = Math.max(0, status.resumeAtMs - Date.now());
    if (remaining <= 0) {
      pauseCountdown = '';
      refreshEngineStatus().catch(() => {});
      return;
    }
    const totalSec = Math.floor(remaining / 1000);
    const min = Math.floor(totalSec / 60);
    const sec = totalSec % 60;
    pauseCountdown = `${min}:${sec.toString().padStart(2, '0')}`;
  }

  function planName(guid: string) {
    const match = plans.find((plan) => plan.guid.toLowerCase() === guid.toLowerCase());
    return match ? match.name : 'Unknown';
  }

  onMount(() => {
    getVersion().then((v) => { version = v; }).catch(() => {});
    loadInitialData().catch((error) => {
      errorText = error instanceof Error ? error.message : String(error);
    });

    sampleUsage();
    const timer = window.setInterval(() => {
      sampleUsage();
      refreshEngineStatus().catch(() => {});
      updateCountdown();
    }, 1000);

    return () => {
      window.clearInterval(timer);
    };
  });
</script>

<!-- Background glow layers — one per theme, cross-faded via opacity -->
<div class="bg-glow bg-balanced" style="opacity:{activeCategory === 'balanced' ? 1 : 0}"></div>
<div class="bg-glow bg-saver"   style="opacity:{activeCategory === 'saver'   ? 1 : 0}"></div>
<div class="bg-glow bg-perf"    style="opacity:{activeCategory === 'performance' ? 1 : 0}"></div>

<header data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <svg class="brand-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M13 2L4.5 13.5H11L11 22L19.5 10.5H13L13 2Z" fill="currentColor" />
    </svg>
    <h1 data-tauri-drag-region>Power Plan Pro</h1>
    <span class="version-chip" data-tauri-drag-region>v{version}</span>
  </div>
  <div class="header-right" data-tauri-drag-region>
    <div class="plan-badge plan-badge--{activeCategory}" data-tauri-drag-region>
      <span class="badge-dot"></span>
      {status ? planName(status.activePlanGuid) : 'Loading…'}
    </div>
  </div>
  <div class="win-controls">
    <button class="wc-btn wc-min" title="Minimize" on:click={() => appWindow.minimize()}>
      <svg viewBox="0 0 10 1" fill="currentColor"><rect width="10" height="1"/></svg>
    </button>
    <button class="wc-btn wc-max" title="Maximize" on:click={() => appWindow.toggleMaximize()}>
      <svg viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1"><rect x="0.5" y="0.5" width="9" height="9"/></svg>
    </button>
    <button class="wc-btn wc-close" title="Close" on:click={() => appWindow.close()}>
      <svg viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><line x1="0" y1="0" x2="10" y2="10"/><line x1="10" y1="0" x2="0" y2="10"/></svg>
    </button>
  </div>
</header>

<main>

  <!-- Metric cards -->
  <div class="metrics-row">
    <div class="card metric-card">
      <span class="metric-label">CPU</span>
      <span class="metric-value">{cpuUsage.toFixed(1)}<small>%</small></span>
      <div class="metric-bar">
        <div class="metric-fill cpu-fill" style="width:{Math.min(cpuUsage, 100).toFixed(2)}%"></div>
      </div>
      <span class="metric-sub">avg {status?.avgCpu.toFixed(1) ?? '—'}%</span>
    </div>
    <div class="card metric-card">
      <span class="metric-label">GPU</span>
      <span class="metric-value">{gpuUsage.toFixed(1)}<small>%</small></span>
      <div class="metric-bar">
        <div class="metric-fill gpu-fill" style="width:{Math.min(gpuUsage, 100).toFixed(2)}%"></div>
      </div>
      <span class="metric-sub">avg {status?.avgGpu.toFixed(1) ?? '—'}%</span>
    </div>
    <div class="card metric-card">
      <span class="metric-label">Rule Engine</span>
      {#if status?.paused}
        <span class="engine-badge badge-paused">⏸ Paused</span>
        <span class="metric-sub">{pauseCountdown} remaining</span>
      {:else if config.enabled}
        {#if status?.conditionMet}
          <span class="engine-badge badge-switching">● Triggered</span>
          <span class="metric-sub">Switches: {status?.switchesCount ?? 0}</span>
        {:else}
          <span class="engine-badge badge-watching">○ Monitoring</span>
          <span class="metric-sub">Switches: {status?.switchesCount ?? 0}</span>
        {/if}
      {:else}
        <span class="engine-badge badge-off">– Off</span>
        <span class="metric-sub">Switches: {status?.switchesCount ?? 0}</span>
      {/if}
    </div>
  </div>

  <!-- Chart -->
  <section class="card graph-panel">
    <div class="panel-header">
      <span class="panel-title">Usage History</span>
      <div class="tf-group">
        {#each TIMEFRAMES as tf}
          <button
            class="tf-btn {viewPoints === tf.points ? 'tf-active' : ''}"
            on:click={() => (viewPoints = tf.points)}
          >{tf.label}</button>
        {/each}
      </div>
      <span class="ts">{lastUpdated}</span>
    </div>
    <svg
      bind:this={svgEl}
      viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
      preserveAspectRatio="none"
      class="chart-svg"
      aria-label="CPU and GPU usage chart"
    >
      <defs>
        <linearGradient id="cpu-grad" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%"  stop-color="#4a9eff" stop-opacity="0.28" />
          <stop offset="100%" stop-color="#4a9eff" stop-opacity="0" />
        </linearGradient>
        <linearGradient id="gpu-grad" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%"  stop-color="#00d4a8" stop-opacity="0.22" />
          <stop offset="100%" stop-color="#00d4a8" stop-opacity="0" />
        </linearGradient>
      </defs>
      <g bind:this={chartGroupEl}>
        {#each planBands as band}
          <rect x={band.x} y={0} width={band.width} height={CHART_HEIGHT} fill={planColor(band.guid)} />
        {/each}
        <line x1="0" y1={CHART_HEIGHT * 0.25} x2={CHART_WIDTH} y2={CHART_HEIGHT * 0.25} class="grid-line" />
        <line x1="0" y1={CHART_HEIGHT * 0.5}  x2={CHART_WIDTH} y2={CHART_HEIGHT * 0.5}  class="grid-line" />
        <line x1="0" y1={CHART_HEIGHT * 0.75} x2={CHART_WIDTH} y2={CHART_HEIGHT * 0.75} class="grid-line" />
        <path d={toAreaPath(gpuView, viewPoints)} class="area gpu-area" />
        <path d={toAreaPath(cpuView, viewPoints)} class="area cpu-area" />
        <path d={toPath(gpuView, viewPoints)} class="line gpu-line" />
        <path d={toPath(cpuView, viewPoints)} class="line cpu-line" />
      </g>
    </svg>
    <div class="legend">
      <span><i class="dot cpu-dot"></i>CPU</span>
      <span><i class="dot gpu-dot"></i>GPU</span>
      {#each seenPlanGuids as guid}
        <span><i class="plan-swatch" style="background:{planColor(guid)}"></i>{planName(guid)}</span>
      {/each}
    </div>
  </section>

  <!-- Config -->
  <section class="card config-panel">
    <div class="panel-header">
      <span class="panel-title">Rule Engine</span>
      <label class="toggle" title="{config.enabled ? 'Disable' : 'Enable'} rule engine">
        <input type="checkbox" bind:checked={config.enabled} on:change={saveConfig} />
        <span class="track"><span class="thumb"></span></span>
      </label>
    </div>

    <!-- Plain-English trigger summary -->
    <p class="rule-summary">
      Switch to high-load plan when
      <span class="mode-toggle">
        <button class="mode-btn {config.conditionMode === 'or'  ? 'mode-active' : ''}" on:click={() => { config.conditionMode = 'or';  saveConfig(); }}>CPU or GPU</button>
        <button class="mode-btn {config.conditionMode === 'and' ? 'mode-active' : ''}" on:click={() => { config.conditionMode = 'and'; saveConfig(); }}>CPU and GPU</button>
      </span>
      exceeds threshold for
      <input class="inline-num" type="number" min="1" max="900" step="1" bind:value={config.durationSeconds} on:change={saveConfig} />
      s, then hold for
      <input class="inline-num" type="number" min="0" max="3600" step="1" bind:value={config.cooldownSeconds} on:change={saveConfig} />
      s.
    </p>

    <div class="cfg-groups">
      <!-- Thresholds -->
      <div class="cfg-group">
        <span class="cfg-group-label">
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M8 2v12M4 10l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
          Thresholds
        </span>
        <div class="cfg-row">
          <div class="cfg-slider-field">
            <div class="cfg-slider-header">
              <span class="cfg-icon cpu-col">CPU</span>
              <span class="cfg-slider-val cpu-col">{config.cpuThreshold}%</span>
            </div>
            <input class="cfg-slider cpu-slider" type="range" min="0" max="100" step="1"
              bind:value={config.cpuThreshold} on:change={saveConfig}
              style="--pct:{config.cpuThreshold}%" />
          </div>
          <div class="cfg-slider-field">
            <div class="cfg-slider-header">
              <span class="cfg-icon gpu-col">GPU</span>
              <span class="cfg-slider-val gpu-col">{config.gpuThreshold}%</span>
            </div>
            <input class="cfg-slider gpu-slider" type="range" min="0" max="100" step="1"
              bind:value={config.gpuThreshold} on:change={saveConfig}
              style="--pct:{config.gpuThreshold}%" />
          </div>
        </div>
      </div>

      <!-- Plans -->
      <div class="cfg-group">
        <span class="cfg-group-label">
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.5"/><path d="M8 5.5V8l2 1.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
          Plans
        </span>
        <div class="cfg-row">
          <div class="cfg-field">
            <span class="cfg-icon perf-col">High</span>
            <div class="cdd-wrap">
              <button class="cdd-trigger perf-cdd" on:click={() => { highDropOpen = !highDropOpen; lowDropOpen = false; }}>
                <span>{planName(config.highLoadPlanGuid)}</span>
                <svg class="cdd-chevron {highDropOpen ? 'open' : ''}" viewBox="0 0 10 6" fill="none"><path d="M1 1l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              {#if highDropOpen}
                <div class="cdd-menu">
                  {#each plans as plan}
                    <button class="cdd-item {config.highLoadPlanGuid === plan.guid ? 'cdd-selected' : ''}" on:click={() => selectHigh(plan.guid)}>{plan.name}</button>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
          <div class="cfg-field">
            <span class="cfg-icon saver-col">Low</span>
            <div class="cdd-wrap">
              <button class="cdd-trigger saver-cdd" on:click={() => { lowDropOpen = !lowDropOpen; highDropOpen = false; }}>
                <span>{planName(config.lowLoadPlanGuid)}</span>
                <svg class="cdd-chevron {lowDropOpen ? 'open' : ''}" viewBox="0 0 10 6" fill="none"><path d="M1 1l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
              {#if lowDropOpen}
                <div class="cdd-menu">
                  {#each plans as plan}
                    <button class="cdd-item {config.lowLoadPlanGuid === plan.guid ? 'cdd-selected' : ''}" on:click={() => selectLow(plan.guid)}>{plan.name}</button>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="actions">
      {#each plans.slice(0, 3) as plan}
        <button class="btn" on:click={() => setPlan(plan.guid)}>Apply {plan.name}</button>
      {/each}
    </div>

    <!-- Pause Rule Engine -->
    <div class="pause-row">
      <div class="pause-header">
        <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M3 2h4v12H3V2zm6 0h4v12H9V2z"/></svg>
        <span class="pause-label">Pause Rule Engine</span>
        {#if status?.paused}
          <span class="pause-active-badge">{pauseCountdown}</span>
        {/if}
      </div>
      <div class="pause-controls">
        <input
          class="pause-input"
          type="number"
          min="1"
          step="1"
          bind:value={pauseMinutes}
          disabled={status?.paused}
        />
        <span class="pause-unit">min</span>
        <input
          class="pause-slider"
          type="range"
          min="1"
          max="30"
          step="1"
          bind:value={pauseMinutes}
          disabled={status?.paused}
          style="--pct:{((pauseMinutes - 1) / 29) * 100}%"
        />
        {#if status?.paused}
          <button class="btn btn-pause-cancel" on:click={cancelPause}>Cancel</button>
        {:else}
          <button class="btn btn-pause" on:click={applyPause}>Pause</button>
        {/if}
      </div>
    </div>

    <label class="autostart-row">
      <span class="autostart-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M1 1h6.5v6.5H1V1zm7.5 0H15v6.5H8.5V1zM1 8.5h6.5V15H1V8.5zm7.5 0H15V15H8.5V8.5z" fill="currentColor"/>
        </svg>
      </span>
      <span class="autostart-label">Start with Windows</span>
      <input type="checkbox" bind:checked={autostartEnabled} on:change={applyAutostart} />
      <span class="track"><span class="thumb"></span></span>
    </label>
  </section>

  {#if errorText}
    <div class="error-banner">{errorText}</div>
  {/if}
</main>

<style>
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  :global(body) {
    font-family: 'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif;
    background: #070b12;
    color: #dde8f5;
    min-height: 100vh;
  }

  /* ── Background glow layers ─────────────────── */
  .bg-glow {
    position: fixed;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    transition: opacity 1.8s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .bg-balanced {
    background: radial-gradient(ellipse 100% 55% at 50% -5%, rgba(40, 110, 255, 0.55) 0%, rgba(20, 60, 180, 0.18) 45%, transparent 70%);
  }

  .bg-saver {
    background: radial-gradient(ellipse 100% 55% at 50% -5%, rgba(0, 210, 100, 0.55) 0%, rgba(0, 130, 70, 0.18) 45%, transparent 70%);
  }

  .bg-perf {
    background: radial-gradient(ellipse 100% 55% at 50% -5%, rgba(255, 50, 20, 0.55) 0%, rgba(200, 30, 10, 0.18) 45%, transparent 70%);
  }

  /* ── Layout ─────────────────────────────────── */
  main {
    max-width: 860px;
    margin: 0 auto;
    padding: 16px 18px 36px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    position: relative;
    z-index: 1;
  }

  /* ── Glass cards ─────────────────────────────── */
  .card {
    background: rgba(255, 255, 255, 0.045);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 16px;
    padding: 16px 18px;
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    transition: box-shadow 0.3s ease;
  }

  .card:hover {
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.11), 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  /* ── Titlebar ────────────────────────────────── */
  header {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 8px 0 16px;
    background: rgba(255, 255, 255, 0.045);
    border-bottom: 1px solid rgba(255, 255, 255, 0.09);
    backdrop-filter: blur(28px) saturate(1.4);
    -webkit-backdrop-filter: blur(28px) saturate(1.4);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.05), 0 4px 24px rgba(0, 0, 0, 0.35);
    user-select: none;
  }

  :global(body) {
    padding-top: 44px;
  }

  .win-controls {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: 12px;
    flex-shrink: 0;
  }

  .wc-btn {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    border: none;
    background: rgba(255, 255, 255, 0.05);
    color: #6a8aaa;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease;
    padding: 0;
    flex-shrink: 0;
  }

  .wc-btn svg {
    width: 10px;
    height: 10px;
  }

  .wc-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #c8ddf0;
  }

  .wc-close:hover {
    background: rgba(220, 50, 40, 0.7);
    color: #fff;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .brand-icon {
    width: 20px;
    height: 20px;
    color: #4a9eff;
    flex-shrink: 0;
  }

  h1 {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.3px;
    color: #e4eefa;
  }

  .header-right {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    pointer-events: none;
  }

  .header-right > * {
    pointer-events: auto;
  }

  .plan-badge {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 5px 13px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
    border: 1px solid transparent;
    transition: background 1.8s ease, color 1.8s ease, border-color 1.8s ease;
  }

  .plan-badge--balanced {
    background: rgba(40, 110, 255, 0.18);
    color: #7eb6ff;
    border-color: rgba(74, 158, 255, 0.3);
  }

  .plan-badge--saver {
    background: rgba(0, 200, 100, 0.18);
    color: #5de3a8;
    border-color: rgba(0, 200, 100, 0.3);
  }

  .plan-badge--performance {
    background: rgba(255, 55, 20, 0.18);
    color: #ff8f6a;
    border-color: rgba(255, 55, 20, 0.3);
  }

  .badge-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    animation: pulse 2.2s ease-in-out infinite;
  }

  .plan-badge--balanced .badge-dot    { background: #4a9eff; }
  .plan-badge--saver .badge-dot       { background: #00d484; }
  .plan-badge--performance .badge-dot { background: #ff5533; }

  @keyframes pulse {
    0%, 100% { opacity: 1;   transform: scale(1); }
    50%       { opacity: 0.4; transform: scale(0.7); }
  }

  .version-chip {
    font-size: 11px;
    font-weight: 600;
    color: #7aa8d4;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid rgba(122, 168, 212, 0.25);
    background: rgba(122, 168, 212, 0.08);
    align-self: center;
  }

  /* ── Metrics row ─────────────────────────────── */
  .metrics-row {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 12px;
  }

  .metric-card {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .metric-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #445870;
  }

  .metric-value {
    font-size: 36px;
    font-weight: 800;
    letter-spacing: -1.5px;
    color: #e4eefa;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  .metric-value small {
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.5px;
    opacity: 0.5;
  }

  .metric-bar {
    height: 3px;
    background: rgba(255, 255, 255, 0.07);
    border-radius: 2px;
    overflow: hidden;
  }

  .metric-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.85s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .cpu-fill { background: linear-gradient(90deg, #2461d8, #4a9eff); }
  .gpu-fill { background: linear-gradient(90deg, #009e7a, #00d4a8); }

  .metric-sub {
    font-size: 11px;
    color: #344d66;
  }

  .engine-badge {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.02em;
    transition: color 0.4s ease;
  }

  .badge-switching { color: #ff8f6a; }
  .badge-watching  { color: #4a9eff; }
  .badge-off       { color: #2e4258; }

  /* ── Graph panel ─────────────────────────────── */
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }

  .panel-title {
    font-size: 13px;
    font-weight: 600;
    color: #7a9ab8;
  }

  .tf-group {
    display: flex;
    gap: 3px;
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 8px;
    padding: 3px;
  }

  .tf-btn {
    cursor: pointer;
    background: transparent;
    border: none;
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 11px;
    font-weight: 600;
    color: #3d5570;
    transition: background 0.18s ease, color 0.18s ease;
    letter-spacing: 0.02em;
  }

  .tf-btn:hover {
    color: #7a9ab8;
    background: rgba(255, 255, 255, 0.06);
    box-shadow: none;
    transform: none;
  }

  .tf-active {
    background: rgba(74, 158, 255, 0.18) !important;
    color: #7eb6ff !important;
    border: none;
  }

  .ts {
    font-size: 11px;
    color: #2e4258;
  }

  .chart-svg {
    width: 100%;
    height: 150px;
    display: block;
    background: rgba(0, 0, 0, 0.28);
    border-radius: 10px;
    border: 1px solid rgba(255, 255, 255, 0.05);
  }

  .grid-line {
    stroke: rgba(255, 255, 255, 0.045);
    stroke-width: 0.5;
    vector-effect: non-scaling-stroke;
  }

  .plan-label {
    font-size: 9px;
    fill: rgba(255, 255, 255, 0.28);
  }

  .line {
    fill: none;
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }

  .area {
    stroke: none;
    vector-effect: non-scaling-stroke;
  }

  .cpu-line { stroke: #4a9eff; }
  .gpu-line { stroke: #00d4a8; }
  .cpu-area { fill: url(#cpu-grad); }
  .gpu-area { fill: url(#gpu-grad); }

  .val-label {
    font-size: 10px;
    dominant-baseline: middle;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .cpu-val { fill: #4a9eff; }
  .gpu-val { fill: #00d4a8; }

  .legend {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 14px;
    margin-top: 10px;
    font-size: 12px;
    color: #3d5570;
  }

  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 5px;
    vertical-align: middle;
  }

  .cpu-dot { background: #4a9eff; }
  .gpu-dot { background: #00d4a8; }

  .plan-swatch {
    display: inline-block;
    width: 12px;
    height: 8px;
    border-radius: 2px;
    margin-right: 5px;
    vertical-align: middle;
    border: 1px solid rgba(255, 255, 255, 0.12);
  }

  /* ── Config panel ────────────────────────────── */
  .rule-summary {
    font-size: 13px;
    color: #6a8aaa;
    line-height: 2.2;
    margin-bottom: 16px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .mode-toggle {
    display: inline-flex;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.09);
    border-radius: 8px;
    padding: 2px;
    gap: 2px;
  }

  .mode-btn {
    padding: 3px 10px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 6px;
    border: none;
    background: transparent;
    color: #5a7898;
    cursor: pointer;
    transition: background 0.18s ease, color 0.18s ease;
    white-space: nowrap;
  }

  .mode-btn:hover {
    background: rgba(255, 255, 255, 0.07);
    color: #a0c4e8;
  }

  .mode-active {
    background: rgba(74, 158, 255, 0.22) !important;
    color: #7ec4ff !important;
    border: 1px solid rgba(74, 158, 255, 0.35);
  }

  .inline-num {
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 6px;
    padding: 2px 7px;
    font-size: 13px;
    color: #c8ddf0;
    outline: none;
    transition: border-color 0.2s ease;
    appearance: textfield;
    -moz-appearance: textfield;
    width: 52px;
    text-align: center;
  }

  .inline-num::-webkit-outer-spin-button,
  .inline-num::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .inline-num:focus {
    border-color: rgba(74, 158, 255, 0.5);
    background: rgba(74, 158, 255, 0.08);
  }

  .cfg-groups {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-bottom: 16px;
  }

  .cfg-group {
    background: rgba(0, 0, 0, 0.18);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .cfg-group-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #3a5272;
  }

  .cfg-group-label svg {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  .cfg-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .cfg-field {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .cfg-icon {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    width: 30px;
    flex-shrink: 0;
    text-align: right;
  }

  .cpu-col   { color: #4a9eff; }
  .gpu-col   { color: #00d4a8; }
  .perf-col  { color: #ff8060; }
  .saver-col { color: #4ad490; }

  .cdd-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .cdd-trigger {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    font-size: 12px;
    font-weight: 500;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.09);
    background: rgba(255, 255, 255, 0.055);
    color: #b8cce0;
    cursor: pointer;
    transition: border-color 0.2s ease, background 0.2s ease;
    text-align: left;
  }

  .cdd-trigger:hover {
    background: rgba(255, 255, 255, 0.09);
    border-color: rgba(255, 255, 255, 0.15);
  }

  .perf-cdd:focus-visible {
    border-color: rgba(255, 128, 96, 0.5);
    box-shadow: 0 0 0 3px rgba(255, 128, 96, 0.1);
  }

  .saver-cdd:focus-visible {
    border-color: rgba(74, 212, 144, 0.5);
    box-shadow: 0 0 0 3px rgba(74, 212, 144, 0.1);
  }

  .cdd-chevron {
    width: 10px;
    height: 6px;
    color: #3a5272;
    flex-shrink: 0;
    transition: transform 0.2s ease;
  }

  .cdd-chevron.open {
    transform: rotate(180deg);
  }

  .cdd-menu {
    position: absolute;
    top: calc(100% + 5px);
    left: 0;
    right: 0;
    z-index: 200;
    background: rgba(14, 22, 38, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 10px;
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .cdd-item {
    width: 100%;
    text-align: left;
    padding: 6px 10px;
    font-size: 12px;
    font-weight: 500;
    border-radius: 6px;
    border: none;
    background: transparent;
    color: #7a9bb8;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .cdd-item:hover {
    background: rgba(255, 255, 255, 0.08);
    color: #c8ddf0;
  }

  .cdd-selected {
    background: rgba(74, 158, 255, 0.15);
    color: #7ec4ff;
    font-weight: 700;
  }

  .cfg-unit {
    font-size: 12px;
    color: #3a5272;
    font-weight: 600;
  }

  .cfg-slider-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .cfg-slider-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .cfg-slider-val {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.03em;
    min-width: 32px;
    text-align: right;
  }

  .cfg-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    border-radius: 999px;
    outline: none;
    cursor: pointer;
    border: none;
    background: linear-gradient(
      to right,
      var(--track-fill) 0%,
      var(--track-fill) var(--pct),
      rgba(255, 255, 255, 0.1) var(--pct),
      rgba(255, 255, 255, 0.1) 100%
    );
  }

  .cpu-slider { --track-fill: #4a9eff; }
  .gpu-slider { --track-fill: #00d4a8; }

  .cfg-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #c8ddf0;
    border: 2px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
    transition: transform 0.15s ease, background 0.15s ease;
  }

  .cfg-slider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
    background: #fff;
  }

  .cfg-slider::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #c8ddf0;
    border: 2px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
    cursor: pointer;
  }

  /* ── Toggle switch ───────────────────────────── */
  .toggle {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .track {
    display: inline-flex;
    align-items: center;
    width: 40px;
    height: 22px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 999px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    transition: background 0.25s ease, border-color 0.25s ease;
    position: relative;
  }

  .toggle input:checked ~ .track {
    background: rgba(74, 158, 255, 0.35);
    border-color: rgba(74, 158, 255, 0.5);
  }

  .thumb {
    position: absolute;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #5a7898;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
    transition: transform 0.25s cubic-bezier(0.34, 1.56, 0.64, 1), background 0.25s ease;
  }

  .toggle input:checked ~ .track .thumb {
    transform: translateX(18px);
    background: #4a9eff;
  }

  /* ── Buttons ─────────────────────────────────── */
  .actions {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }

  .btn {
    cursor: pointer;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 9px;
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 600;
    color: #7898b8;
    background: rgba(255, 255, 255, 0.055);
    transition:
      background 0.2s ease,
      border-color 0.2s ease,
      color 0.2s ease,
      transform 0.15s ease,
      box-shadow 0.2s ease;
    letter-spacing: 0.01em;
  }

  .btn:hover {
    background: rgba(255, 255, 255, 0.1);
    border-color: rgba(255, 255, 255, 0.18);
    color: #c8ddf0;
    transform: translateY(-1px);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  }

  .btn:active {
    transform: translateY(0);
    box-shadow: none;
  }

  .btn-primary {
    background: rgba(74, 158, 255, 0.18);
    border-color: rgba(74, 158, 255, 0.32);
    color: #7eb6ff;
  }

  .btn-primary:hover {
    background: rgba(74, 158, 255, 0.28);
    border-color: rgba(74, 158, 255, 0.5);
    color: #b0d4ff;
    box-shadow: 0 4px 20px rgba(74, 158, 255, 0.18);
  }

  /* ── Pause Rule Engine ───────────────────────── */
  .badge-paused { color: #e0a040; }

  .pause-row {
    background: rgba(0, 0, 0, 0.18);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 14px;
  }

  .pause-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #3a5272;
  }

  .pause-header svg {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  .pause-label {
    flex: 1;
  }

  .pause-active-badge {
    font-size: 11px;
    font-weight: 700;
    color: #e0a040;
    background: rgba(224, 160, 64, 0.15);
    border: 1px solid rgba(224, 160, 64, 0.3);
    border-radius: 6px;
    padding: 2px 8px;
    letter-spacing: 0.05em;
    font-variant-numeric: tabular-nums;
  }

  .pause-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pause-input {
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 6px;
    padding: 4px 7px;
    font-size: 13px;
    color: #c8ddf0;
    outline: none;
    transition: border-color 0.2s ease;
    appearance: textfield;
    -moz-appearance: textfield;
    width: 46px;
    text-align: center;
  }

  .pause-input::-webkit-outer-spin-button,
  .pause-input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .pause-input:focus {
    border-color: rgba(224, 160, 64, 0.5);
    background: rgba(224, 160, 64, 0.08);
  }

  .pause-input:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .pause-unit {
    font-size: 12px;
    color: #3a5272;
    font-weight: 600;
  }

  .pause-slider {
    -webkit-appearance: none;
    appearance: none;
    flex: 1;
    height: 4px;
    border-radius: 999px;
    outline: none;
    cursor: pointer;
    border: none;
    background: linear-gradient(
      to right,
      #e0a040 0%,
      #e0a040 var(--pct),
      rgba(255, 255, 255, 0.1) var(--pct),
      rgba(255, 255, 255, 0.1) 100%
    );
  }

  .pause-slider:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .pause-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #c8ddf0;
    border: 2px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
    transition: transform 0.15s ease, background 0.15s ease;
  }

  .pause-slider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
    background: #fff;
  }

  .pause-slider::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #c8ddf0;
    border: 2px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
    cursor: pointer;
  }

  .pause-slider:disabled::-webkit-slider-thumb {
    background: #5a7898;
    cursor: not-allowed;
  }

  .pause-slider:disabled::-moz-range-thumb {
    background: #5a7898;
    cursor: not-allowed;
  }

  .btn-pause {
    background: rgba(224, 160, 64, 0.18);
    border-color: rgba(224, 160, 64, 0.32);
    color: #e0a040;
  }

  .btn-pause:hover {
    background: rgba(224, 160, 64, 0.28);
    border-color: rgba(224, 160, 64, 0.5);
    color: #f0c060;
    box-shadow: 0 4px 20px rgba(224, 160, 64, 0.18);
  }

  .btn-pause-cancel {
    background: rgba(220, 60, 40, 0.18);
    border-color: rgba(220, 60, 40, 0.32);
    color: #ff8060;
  }

  .btn-pause-cancel:hover {
    background: rgba(220, 60, 40, 0.28);
    border-color: rgba(220, 60, 40, 0.5);
    color: #ffa080;
    box-shadow: 0 4px 20px rgba(220, 60, 40, 0.18);
  }

  /* ── Autostart ───────────────────────────────── */
  .autostart-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 12px;
    cursor: pointer;
    transition: background 0.2s ease, border-color 0.2s ease;
  }

  .autostart-row:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.12);
  }

  .autostart-icon {
    display: flex;
    align-items: center;
    color: #3a5272;
    flex-shrink: 0;
    transition: color 0.2s ease;
  }

  .autostart-icon svg {
    width: 14px;
    height: 14px;
  }

  .autostart-row:has(input:checked) .autostart-icon {
    color: #4a9eff;
  }

  .autostart-label {
    flex: 1;
    font-size: 13px;
    font-weight: 500;
    color: #5a7898;
    transition: color 0.2s ease;
  }

  .autostart-row:has(input:checked) .autostart-label {
    color: #a0c4e8;
  }

  .autostart-row input[type='checkbox'] {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .autostart-row input:checked ~ .track {
    background: rgba(74, 158, 255, 0.35);
    border-color: rgba(74, 158, 255, 0.5);
  }

  .autostart-row input:checked ~ .track .thumb {
    transform: translateX(18px);
    background: #4a9eff;
  }

  /* ── Error ───────────────────────────────────── */
  .error-banner {
    background: rgba(220, 50, 30, 0.14);
    border: 1px solid rgba(220, 60, 30, 0.28);
    border-radius: 12px;
    padding: 10px 14px;
    font-size: 13px;
    color: #ff9070;
  }
</style>
