<script>
  import { Copy, Check } from '@lucide/svelte'
  import { copyText } from './copy.js'
  // A small Copy button for error text (for bug reports and searches).
  let { text, label = 'Copy' } = $props()
  let done = $state(false)
  async function copy() {
    done = await copyText(text)
    if (done) setTimeout(() => (done = false), 1500)
  }
</script>

<button type="button" class="btn ghost sm copy" onclick={copy} title="Copy the full message" aria-label="Copy the full message">
  {#if done}<Check size={14} /> Copied{:else}<Copy size={14} /> {label}{/if}
</button>

<style>
  .copy { flex: none; margin-left: auto; color: inherit; }
</style>
