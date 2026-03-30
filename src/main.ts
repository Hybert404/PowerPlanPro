import { mount } from 'svelte'
import App from './App.svelte'

function renderFatalError(message: string) {
  const container = document.getElementById('app')
  if (!container) {
    return
  }

  container.innerHTML = `
    <div style="font-family: Segoe UI, sans-serif; padding: 20px; color: #1f2d3d;">
      <h2 style="margin: 0 0 8px 0;">Power Plan Pro failed to start</h2>
      <pre style="white-space: pre-wrap; background: #f5f7fa; border: 1px solid #d9e1ec; border-radius: 8px; padding: 12px;">${message}</pre>
    </div>
  `
}

window.addEventListener('error', (event) => {
  const details = event.error instanceof Error ? event.error.stack ?? event.error.message : String(event.message)
  renderFatalError(details)
})

window.addEventListener('unhandledrejection', (event) => {
  const reason = event.reason
  const details = reason instanceof Error ? reason.stack ?? reason.message : String(reason)
  renderFatalError(details)
})

let app: ReturnType<typeof mount>

try {
  app = mount(App, {
    target: document.getElementById('app')!,
  })
} catch (error) {
  const details = error instanceof Error ? error.stack ?? error.message : String(error)
  renderFatalError(details)
  throw error
}

export default app
