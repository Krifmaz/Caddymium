<script>
  import { CircleX } from '@lucide/svelte'
  import CopyButton from './CopyButton.svelte'
  // An error, in full and selectable, with Copy.
  let { message, title = '', class: cls = '' } = $props()
  const full = $derived(title ? `${title}\n${message}` : message)
</script>

{#if message}
  <div class="alert err errbox {cls}" role="alert">
    <CircleX />
    <div class="grow body">{#if title}<strong>{title}</strong><br />{/if}{message}</div>
    <CopyButton text={full} />
  </div>
{/if}

<style>
  .errbox { align-items: flex-start; }
  .body { white-space: pre-wrap; word-break: break-word; user-select: text; min-width: 0; }
</style>
