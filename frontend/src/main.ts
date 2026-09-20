import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
mount(App, { target: document.getElementById('app')! });
if (
  !('__TAURI_INTERNALS__' in window) &&
  'serviceWorker' in navigator &&
  window.isSecureContext
) {
  void navigator.serviceWorker
    .register('/sw.js', { updateViaCache: 'none' })
    .catch(() => {
      // Browser playback remains available when installation is disabled by policy.
    });
}
