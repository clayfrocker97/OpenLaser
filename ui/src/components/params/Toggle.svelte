<script lang="ts">
  // A labelled on/off switch.
  let { label, value, disabled = false, apply }: {
    label: string;
    value: boolean;
    disabled?: boolean;
    apply: (v: boolean) => void;
  } = $props();
</script>

<div class="param">
  <div class="lbl">{label}</div>
  <button class="switch" class:on={value} onclick={() => apply(!value)} {disabled} style="justify-self:end" aria-label={label}></button>
</div>
