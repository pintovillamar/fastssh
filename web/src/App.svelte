<script lang="ts">
  import Home from './Home.svelte'
  import Terminal from './Terminal.svelte'
  import type { SavedConnection } from './api'

  interface Tab {
    id: number
    title: string
    connectionId: number | null
  }

  let tabs = $state<Tab[]>([])
  // null shows the connection list.
  let active = $state<number | null>(null)
  let nextId = 1

  function open(connection: SavedConnection | null) {
    const tab = {
      id: nextId++,
      title: connection?.name ?? 'Local shell',
      connectionId: connection?.id ?? null,
    }
    tabs.push(tab)
    active = tab.id
  }

  function close(id: number) {
    const index = tabs.findIndex((tab) => tab.id === id)
    tabs.splice(index, 1)
    if (active === id) active = (tabs[index] ?? tabs[index - 1])?.id ?? null
  }
</script>

<div class="app">
  {#if tabs.length > 0}
    <nav>
      {#each tabs as tab (tab.id)}
        <div class="tab" class:active={tab.id === active}>
          <button class="plain title" onclick={() => (active = tab.id)}>{tab.title}</button>
          <button class="plain close" aria-label="Close {tab.title}" onclick={() => close(tab.id)}>×</button>
        </div>
      {/each}
      <button class="plain new" class:active={active === null} aria-label="New session" onclick={() => (active = null)}>+</button>
    </nav>
  {/if}

  <main>
    <!-- Terminals stay mounted while hidden so their sessions keep running. -->
    {#each tabs as tab (tab.id)}
      <div class="pane" hidden={tab.id !== active}>
        <Terminal connectionId={tab.connectionId} visible={tab.id === active} />
      </div>
    {/each}
    {#if active === null}
      <div class="pane scroll">
        <Home onopen={open} />
      </div>
    {/if}
  </main>
</div>

<style>
  .app {
    height: 100dvh;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    padding: env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom)
      env(safe-area-inset-left);
  }

  nav {
    display: flex;
    gap: 4px;
    padding: 6px;
    border-bottom: 1px solid var(--outline);
    overflow-x: auto;
    scrollbar-width: none;
    flex: none;
  }

  .tab {
    display: flex;
    align-items: center;
    border-radius: var(--radius);
    flex: none;
    color: var(--secondary);
  }

  .tab.active {
    background: var(--surface-active);
    color: var(--text);
  }

  .tab button {
    padding: 7px 10px;
  }

  .tab .title {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-right: 4px;
  }

  .tab .close {
    color: var(--dim);
    padding-left: 6px;
  }

  .tab .close:hover {
    color: var(--text);
  }

  .new {
    flex: none;
    padding: 7px 12px;
    color: var(--secondary);
  }

  .new.active {
    color: var(--accent);
  }

  main {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .pane {
    position: absolute;
    inset: 0;
  }

  .pane.scroll {
    overflow-y: auto;
  }
</style>
