<script lang="ts">
  import { api } from '../api';
  import AutoSaveForm from '../ui/AutoSaveForm.svelte';
  import FormField from '../ui/FormField.svelte';
  import { formControlClass } from '../ui/styles';
  let { id, url, changed } = $props<{
    id: string;
    url: string;
    changed: () => Promise<void>;
  }>();
  let address = $derived(url);
</script>

<section class="work-section settings-section" aria-label="Web access">
  <h3 class="mb-3.25 text-[12px] font-[650]">Web access</h3>
  <p class="mb-3 text-[11px] text-muted">
    Use the existing web address when proxied access is unavailable. The service
    keeps its own login.
  </p>
  <AutoSaveForm
    label="Native service address"
    value={{ url: address }}
    onRevert={(previous) => (address = previous.url)}
    onsave={async (value) => {
      await api(`/admin/managers/${id}/native-url`, 'PUT', value);
      await changed();
    }}
  >
    <FormField
      >Native URL<input
        class={formControlClass}
        type="url"
        bind:value={address}
        placeholder="http://media-host:7878"
        maxlength="2000"
      /></FormField
    >
  </AutoSaveForm>
</section>
