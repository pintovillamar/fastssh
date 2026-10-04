<script lang="ts">
  import ConnectionForm from './ConnectionForm.svelte'
  import { listConnections, signOut, type SavedConnection, type Session } from './api'

  let {
    session,
    onopen,
    onsession,
  }: {
    session: Session
    onopen: (connection: SavedConnection | null) => void
    onsession: () => void
  } = $props()

  async function leave() {
    await signOut().catch(() => {})
    onsession()
  }

  let connections = $state<SavedConnection[]>([])
  let error = $state('')
  let loaded = $state(false)
  // undefined: list is showing. null: creating. Otherwise: editing that one.
  let editing = $state<SavedConnection | null | undefined>(undefined)

  async function refresh() {
    try {
      connections = await listConnections()
      error = ''
    } catch (err) {
      error = (err as Error).message
    }
    loaded = true
  }

  function done() {
    editing = undefined
    refresh()
  }

  refresh()
</script>

<section>
  {#if editing !== undefined}
    <ConnectionForm connection={editing} ondone={done} oncancel={() => (editing = undefined)} />
  {:else}
    <header>
      <h1>FastSSH</h1>
      <button class="primary" onclick={() => (editing = null)}>New connection</button>
    </header>

    {#if error}
      <p class="error">{error}</p>
    {/if}

    <ul>
      {#each connections as connection (connection.id)}
        <li>
          <button class="plain row" onclick={() => onopen(connection)}>
            <span class="name">{connection.name}</span>
            <span class="where">{connection.username}@{connection.host}{connection.port === 22 ? '' : `:${connection.port}`}</span>
          </button>
          <button class="plain edit" onclick={() => (editing = connection)}>Edit</button>
        </li>
      {/each}
      {#if session.local_shell}
        <li>
          <button class="plain row" onclick={() => onopen(null)}>
            <span class="name">Local shell</span>
            <span class="where">A shell on the machine running FastSSH</span>
          </button>
        </li>
      {/if}
    </ul>

    {#if loaded && connections.length === 0 && !error}
      <p class="hint">No saved connections yet.</p>
    {/if}

    <footer>
      <span>{session.desktop ? '' : session.email}</span>
      <button class="plain" onclick={leave}>{session.desktop ? 'Lock' : 'Sign out'}</button>
    </footer>
  {/if}
</section>

<style>
  section {
    max-width: 640px;
    margin: 0 auto;
    padding: 24px 16px 48px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 20px;
  }

  h1 {
    font-size: 20px;
    font-weight: 650;
    margin: 0;
    letter-spacing: -0.01em;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }

  li {
    display: flex;
    align-items: stretch;
    background: var(--surface);
    border: 1px solid var(--outline);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .row {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 3px;
    text-align: left;
    padding: 14px 16px;
    border-radius: 0;
  }

  .name {
    font-weight: 550;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .where {
    color: var(--secondary);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .edit {
    border-radius: 0;
    color: var(--secondary);
    padding: 0 16px;
  }

  .hint {
    color: var(--dim);
    text-align: center;
    margin-top: 20px;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 28px;
    color: var(--dim);
    font-size: 13px;
  }

  footer span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  footer button {
    color: var(--secondary);
    font-size: 13px;
    flex: none;
  }
</style>
