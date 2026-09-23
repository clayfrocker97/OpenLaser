<script lang="ts">
  // A segmented row that picks one of a few pages.
  let { names, active = $bindable(0) }: { names: string[]; active?: number } = $props();
</script>

<div class="seg block">
  {#each names as name, i}<button class:on={active === i} onclick={() => (active = i)}>{name}</button>{/each}
</div>
