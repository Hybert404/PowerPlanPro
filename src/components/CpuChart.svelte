<script lang="ts">
  export let usage: number = 0;
  
  let canvasElement: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null;
  let dataPoints: number[] = [];
  const MAX_POINTS = 60; // Show last 60 seconds
  
  $: {
    dataPoints = [...dataPoints, usage].slice(-MAX_POINTS);
    drawChart();
  }
  
  function drawChart() {
    if (!canvasElement) return;
    
    const canvas = canvasElement;
    ctx = canvas.getContext('2d');
    if (!ctx) return;
    
    // Clear canvas
    ctx.fillStyle = '#f5f7fa';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    
    if (dataPoints.length < 2) return;
    
    // Draw grid
    ctx.strokeStyle = '#e0e0e0';
    ctx.lineWidth = 1;
    
    for (let i = 0; i <= 4; i++) {
      const y = (canvas.height / 4) * i;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(canvas.width, y);
      ctx.stroke();
    }
    
    // Draw data line
    const padding = 10;
    const graphWidth = canvas.width - 2 * padding;
    const graphHeight = canvas.height - 2 * padding;
    
    ctx.strokeStyle = 'url(#gradient)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    
    dataPoints.forEach((point, index) => {
      const x = padding + (index / (dataPoints.length - 1 || 1)) * graphWidth;
      const y = canvas.height - padding - (point / 100) * graphHeight;
      
      if (index === 0) {
        ctx?.moveTo(x, y);
      } else {
        ctx?.lineTo(x, y);
      }
    });
    
    // Fallback to solid color since we can't use gradients directly
    ctx.strokeStyle = '#667eea';
    ctx.stroke();
    
    // Draw current value indicator
    const lastPoint = dataPoints[dataPoints.length - 1];
    const lastX = padding + graphWidth;
    const lastY = canvas.height - padding - (lastPoint / 100) * graphHeight;
    
    ctx.fillStyle = '#667eea';
    ctx.beginPath();
    ctx.arc(lastX, lastY, 4, 0, Math.PI * 2);
    ctx.fill();
  }
</script>

<canvas bind:this={canvasElement} width="100%" height="150" class="chart"></canvas>

<style>
  .chart {
    width: 100%;
    height: 150px;
    border-radius: 8px;
    background: #f9fafb;
  }
</style>
