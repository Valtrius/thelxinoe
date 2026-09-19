<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  let token = $state(''),
    contact = $state(''),
    configured = $state(false),
    message = $state(''),
    busy = $state(false);
  onMount(() => {
    void api<{ tmdb_configured: boolean; musicbrainz_contact: string | null }>(
      '/admin/metadata',
    )
      .then((v) => {
        configured = v.tmdb_configured;
        contact = v.musicbrainz_contact ?? '';
      })
      .catch((e) => (message = String(e)));
  });
  async function save() {
    busy = true;
    message = '';
    try {
      await api('/admin/metadata', 'PUT', {
        ...(token ? { tmdb_token: token } : {}),
        ...(contact ? { musicbrainz_contact: contact } : {}),
      });
      configured = token ? true : configured;
      token = '';
      message = 'Metadata settings saved.';
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel">
  <h2>Metadata providers</h2>
  <p class="muted">
    One configuration for everyone on this server. Movies and shows use TMDB;
    music uses MusicBrainz and Cover Art Archive.
  </p>
  <form
    class="inline-form"
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <label
      >TMDB API Read Access Token<input
        bind:value={token}
        type="password"
        autocomplete="off"
        placeholder={configured
          ? 'Configured — enter a token to replace it'
          : 'From your TMDB account’s API settings'}
      /></label
    ><label
      >MusicBrainz contact email or URL<input
        bind:value={contact}
        placeholder="Contact for API usage issues"
      /></label
    ><button class="primary" disabled={busy}>Save providers</button>
  </form>
  {#if message}<p role="status" class="muted">{message}</p>{/if}
</section>
<section class="panel">
  <h2>Credits</h2>
  <a href="https://www.themoviedb.org" target="_blank" rel="noreferrer"
    ><img src="/tmdb.svg" width="120" height="16" alt="TMDB" /></a
  >
  <p class="muted">
    This product uses the TMDB API but is not endorsed or certified by TMDB.
  </p>
  <p class="muted">
    Music metadata is provided by <a
      href="https://musicbrainz.org"
      target="_blank"
      rel="noreferrer">MusicBrainz</a
    >. Album artwork is provided by
    <a href="https://coverartarchive.org" target="_blank" rel="noreferrer"
      >Cover Art Archive</a
    >.
  </p>
</section>
