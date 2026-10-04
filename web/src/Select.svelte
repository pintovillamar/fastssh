<script lang="ts" generics="T extends string">
  // A dropdown drawn by us rather than the browser, so it matches the rest of
  // the interface. Behaves like a native select for keyboard and screen readers.
  let {
    value = $bindable(),
    options,
    labelledby,
  }: { value: T; options: { value: T; label: string }[]; labelledby: string } = $props()

  const uid = $props.id()
  let open = $state(false)
  let highlighted = $state(0)
  let root: HTMLDivElement

  const current = $derived(options.find((option) => option.value === value) ?? options[0])

  function show() {
    highlighted = Math.max(0, options.findIndex((option) => option.value === value))
    open = true
  }

  function choose(index: number) {
    value = options[index].value
    open = false
  }

  function onkeydown(event: KeyboardEvent) {
    const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
    if (step !== 0) {
      event.preventDefault()
      if (!open) show()
      else highlighted = (highlighted + step + options.length) % options.length
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      if (open) choose(highlighted)
      else show()
    } else if (event.key === 'Escape' && open) {
      event.preventDefault()
      open = false
    } else if (event.key === 'Tab') {
      open = false
    }
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (open && !root.contains(event.target as Node)) open = false
  }}
/>

<div class="select" bind:this={root}>
  <button
    type="button"
    class="trigger"
    role="combobox"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls="{uid}-list"
    aria-activedescendant={open ? `${uid}-${highlighted}` : undefined}
    aria-labelledby="{labelledby} {uid}-value"
    onclick={() => (open ? (open = false) : show())}
    {onkeydown}
  >
    <span id="{uid}-value">{current.label}</span>
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" class:flipped={open}>
      <path d="M3.5 6l4.5 4.5L12.5 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if open}
    <ul id="{uid}-list" role="listbox" aria-labelledby={labelledby}>
      {#each options as option, index (option.value)}
        <!-- Keyboard use goes through the trigger button, which keeps focus. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="{uid}-{index}"
          role="option"
          aria-selected={option.value === value}
          class:highlighted={index === highlighted}
          onpointermove={() => (highlighted = index)}
          onclick={() => choose(index)}
        >
          {option.label}
          {#if option.value === value}
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select {
    position: relative;
  }

  .trigger {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 12px;
    font-size: 16px;
    text-align: left;
    color: var(--text);
  }

  .trigger svg {
    color: var(--secondary);
    transition: transform 0.12s;
    flex: none;
  }

  .trigger svg.flipped {
    transform: rotate(180deg);
  }

  ul {
    position: absolute;
    z-index: 20;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--surface);
    border: 1px solid var(--outline);
    border-radius: var(--radius);
    box-shadow: 0 12px 32px #000000cc;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 9px 10px;
    border-radius: 7px;
    font-size: 15px;
    color: var(--text);
    cursor: pointer;
  }

  li.highlighted {
    background: var(--surface-active);
  }

  li svg {
    color: var(--accent);
  }
</style>
