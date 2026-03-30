<script lang="ts">
  export let show: boolean = false;

  let config = {
    enableAutoSwitch: false,
    lowCpuThreshold: 30,
    highCpuThreshold: 70,
    lowCpuPlan: 'Power Saver',
    highCpuPlan: 'Balanced',
    enableAutostart: false,
  };

  function saveSettings() {
    console.log('Settings saved:', config);
    localStorage.setItem('powerplanpro-config', JSON.stringify(config));
    show = false;
  }

  function loadSettings() {
    const saved = localStorage.getItem('powerplanpro-config');
    if (saved) {
      config = JSON.parse(saved);
    }
  }

  function closeSettings() {
    show = false;
  }

  function handleOverlayClick(event: MouseEvent) {
    if (event.currentTarget === event.target) {
      closeSettings();
    }
  }

  function handleOverlayKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      closeSettings();
    }
  }

  if (show) {
    loadSettings();
  }
</script>

{#if show}
  <div
    class="settings-overlay"
    role="button"
    tabindex="0"
    aria-label="Close settings"
    on:click={handleOverlayClick}
    on:keydown={handleOverlayKeydown}
  >
    <div class="settings-panel">
      <header>
        <h2>Settings</h2>
        <button class="close-btn" on:click={closeSettings}>✕</button>
      </header>

      <div class="settings-content">
        <div class="setting-group">
          <label>
            <input type="checkbox" bind:checked={config.enableAutoSwitch} />
            <span>Enable Automatic Power Plan Switching</span>
          </label>
          <p class="description">Automatically switch power plans based on CPU usage</p>
        </div>

        {#if config.enableAutoSwitch}
          <div class="setting-group nested">
            <label for="low-threshold">Low CPU Threshold (%)</label>
            <input 
              id="low-threshold"
              type="range" 
              min="5" 
              max="95" 
              bind:value={config.lowCpuThreshold}
              class="slider"
            />
            <div class="value-display">{config.lowCpuThreshold}%</div>
          </div>

          <div class="setting-group nested">
            <label for="high-threshold">High CPU Threshold (%)</label>
            <input 
              id="high-threshold"
              type="range" 
              min="5" 
              max="95" 
              bind:value={config.highCpuThreshold}
              class="slider"
            />
            <div class="value-display">{config.highCpuThreshold}%</div>
          </div>

          <p class="info-text">
            When CPU usage falls below {config.lowCpuThreshold}%, it will switch to Low CPU Plan.<br>
            When CPU usage rises above {config.highCpuThreshold}%, it will switch to High CPU Plan.
          </p>
        {/if}

        <div class="setting-group">
          <label>
            <input type="checkbox" bind:checked={config.enableAutostart} />
            <span>Start with Windows</span>
          </label>
          <p class="description">Launch PowerPlanPro automatically when you sign in</p>
        </div>
      </div>

      <footer>
        <button class="btn btn-secondary" on:click={closeSettings}>Cancel</button>
        <button class="btn btn-primary" on:click={saveSettings}>Save Settings</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .settings-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .settings-panel {
    background: white;
    border-radius: 12px;
    width: 90%;
    max-width: 500px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
  }

  header {
    padding: 20px;
    border-bottom: 1px solid #e0e0e0;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  header h2 {
    margin: 0;
    font-size: 20px;
    color: #333;
  }

  .close-btn {
    background: none;
    border: none;
    font-size: 24px;
    color: #999;
    cursor: pointer;
    padding: 0;
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .close-btn:hover {
    color: #333;
  }

  .settings-content {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
  }

  .setting-group {
    margin-bottom: 24px;
  }

  .setting-group.nested {
    margin-left: 24px;
    padding-left: 16px;
    border-left: 2px solid #e0e0e0;
  }

  label {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    font-size: 14px;
    color: #333;
    margin-bottom: 8px;
  }

  input[type="checkbox"] {
    cursor: pointer;
    width: 18px;
    height: 18px;
  }

  input[type="range"] {
    width: 100%;
    cursor: pointer;
    margin: 8px 0;
  }

  .value-display {
    font-weight: 600;
    color: #667eea;
    font-size: 14px;
  }

  .description, .info-text {
    font-size: 12px;
    color: #999;
    margin: 8px 0 0 0;
  }

  .info-text {
    background: #f5f7fa;
    padding: 12px;
    border-radius: 6px;
    line-height: 1.5;
  }

  footer {
    padding: 20px;
    border-top: 1px solid #e0e0e0;
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }

  .btn {
    padding: 10px 20px;
    border: none;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.3s ease;
  }

  .btn-primary {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
  }

  .btn-primary:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
  }

  .btn-secondary {
    background: #f0f0f0;
    color: #333;
  }

  .btn-secondary:hover {
    background: #e0e0e0;
  }
</style>
