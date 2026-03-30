<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  export let currentPlan: string = '';
  export let powerPlans: string[] = [];
  export let disabled: boolean = false;

  let isChanging = false;

  async function handlePlanChange(event: Event) {
    const target = event.target as HTMLSelectElement;
    const newPlan = target.value;
    
    if (newPlan === currentPlan) return;
    
    isChanging = true;
    try {
      await invoke('set_power_plan', { planName: newPlan });
      currentPlan = newPlan;
    } catch (error) {
      console.error('Failed to set power plan:', error);
      alert('Failed to change power plan. Make sure the app is running as administrator.');
    } finally {
      isChanging = false;
    }
  }
</script>

<select value={currentPlan} on:change={handlePlanChange} {disabled} class="plan-selector">
  <option value="">Select power plan...</option>
  {#each powerPlans as plan (plan)}
    <option value={plan}>{plan}</option>
  {/each}
</select>

{#if isChanging}
  <div class="loading">Changing power plan...</div>
{/if}

<style>
  .plan-selector {
    padding: 10px;
    border: 2px solid #e0e0e0;
    border-radius: 8px;
    font-size: 14px;
    background: white;
    cursor: pointer;
    transition: all 0.3s ease;
  }

  .plan-selector:hover:not(:disabled) {
    border-color: #667eea;
  }

  .plan-selector:focus {
    outline: none;
    border-color: #667eea;
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
  }

  .plan-selector:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .loading {
    font-size: 12px;
    color: #999;
    margin-top: 8px;
    font-style: italic;
  }
</style>
