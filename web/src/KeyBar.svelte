<script lang="ts" module>
  export type BarKey =
    | 'esc'
    | 'tab'
    | 'up'
    | 'down'
    | 'left'
    | 'right'
    | 'home'
    | 'end'
    | 'pageup'
    | 'pagedown'

  /** Modifiers that are armed for the next key only. */
  export interface Modifiers {
    ctrl: boolean
    alt: boolean
  }
</script>

<script lang="ts">
  // The keys a phone keyboard lacks. Shown only on touch devices, directly
  // above the on-screen keyboard.
  let {
    modifiers = $bindable(),
    onkey,
    ontext,
  }: {
    modifiers: Modifiers
    onkey: (key: BarKey) => void
    ontext: (text: string) => void
  } = $props()

  const keys: { key: BarKey; label: string; name: string }[] = [
    { key: 'left', label: '←', name: 'Left' },
    { key: 'down', label: '↓', name: 'Down' },
    { key: 'up', label: '↑', name: 'Up' },
    { key: 'right', label: '→', name: 'Right' },
  ]
  const more: { key: BarKey; label: string }[] = [
    { key: 'home', label: 'Home' },
    { key: 'end', label: 'End' },
    { key: 'pageup', label: 'PgUp' },
    { key: 'pagedown', label: 'PgDn' },
  ]
  // Symbols that sit two layers deep on most phone keyboards.
  const symbols = ['|', '/', '-', '~']

  // Taking focus would close the on-screen keyboard, so presses must not.
  // Cancelling mousedown keeps focus in the terminal; the click still fires.
  function keepFocus(event: Event) {
    event.preventDefault()
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="bar" role="toolbar" tabindex="-1" aria-label="Extra keys" onmousedown={keepFocus}>
  <button tabindex="-1" onclick={() => onkey('esc')}>Esc</button>
  <button tabindex="-1" onclick={() => onkey('tab')}>Tab</button>
  <button tabindex="-1" class:armed={modifiers.ctrl} aria-pressed={modifiers.ctrl} onclick={() => (modifiers.ctrl = !modifiers.ctrl)}>Ctrl</button>
  <button tabindex="-1" class:armed={modifiers.alt} aria-pressed={modifiers.alt} onclick={() => (modifiers.alt = !modifiers.alt)}>Alt</button>
  {#each keys as { key, label, name } (key)}
    <button tabindex="-1" class="arrow" aria-label={name} onclick={() => onkey(key)}>{label}</button>
  {/each}
  {#each symbols as symbol (symbol)}
    <button tabindex="-1" class="symbol" onclick={() => ontext(symbol)}>{symbol}</button>
  {/each}
  {#each more as { key, label } (key)}
    <button tabindex="-1" onclick={() => onkey(key)}>{label}</button>
  {/each}
</div>

<style>
  .bar {
    flex: none;
    display: flex;
    gap: 6px;
    padding: 6px;
    border-top: 1px solid var(--outline);
    overflow-x: auto;
    scrollbar-width: none;
    /* Swiping the bar should scroll it, never zoom or select. */
    touch-action: pan-x;
    user-select: none;
    -webkit-user-select: none;
  }

  button {
    flex: none;
    min-width: 44px;
    height: 38px;
    padding: 0 12px;
    border-radius: 8px;
    font-size: 14px;
    color: var(--secondary);
  }

  button:active {
    background: var(--surface-active);
    color: var(--text);
  }

  .arrow,
  .symbol {
    padding: 0;
    font-size: 16px;
  }

  .symbol {
    font-family: ui-monospace, 'JetBrains Mono', Menlo, monospace;
  }

  .armed {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }

  .armed:active {
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
