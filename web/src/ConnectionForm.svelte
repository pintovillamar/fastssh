<script lang="ts">
  import Select from './Select.svelte'
  import {
    createConnection,
    deleteConnection,
    forgetHostKey,
    updateConnection,
    type ConnectionDetails,
    type SavedConnection,
  } from './api'

  let {
    connection,
    ondone,
    oncancel,
  }: { connection: SavedConnection | null; ondone: () => void; oncancel: () => void } = $props()

  // The form edits a copy; the list only changes once the server accepts it.
  // svelte-ignore state_referenced_locally
  const initial = connection
  let name = $state(initial?.name ?? '')
  let host = $state(initial?.host ?? '')
  let port = $state(initial?.port ?? 22)
  let username = $state(initial?.username ?? '')
  let authKind = $state<'password' | 'key_file'>(initial?.auth.kind ?? 'key_file')
  let keyPath = $state(initial?.auth.kind === 'key_file' ? initial.auth.path : '~/.ssh/id_ed25519')

  let error = $state('')
  let notice = $state('')
  let busy = $state(false)
  let confirmingDelete = $state(false)

  async function attempt(action: () => Promise<unknown>, after: () => void) {
    busy = true
    error = ''
    notice = ''
    try {
      await action()
      after()
    } catch (err) {
      error = (err as Error).message
    } finally {
      busy = false
    }
  }

  function save(event: SubmitEvent) {
    event.preventDefault()
    const details: ConnectionDetails = {
      name,
      host,
      port,
      username,
      auth: authKind === 'password' ? { kind: 'password' } : { kind: 'key_file', path: keyPath },
    }
    attempt(
      () => (initial ? updateConnection(initial.id, details) : createConnection(details)),
      ondone,
    )
  }

  function remove() {
    if (!initial) return
    attempt(() => deleteConnection(initial.id), ondone)
  }

  function forget() {
    if (!initial) return
    attempt(
      () => forgetHostKey(initial.id),
      () => (notice = 'Host key forgotten. You will be asked to trust it on the next connect.'),
    )
  }
</script>

<form onsubmit={save}>
  <h1>{initial ? 'Edit connection' : 'New connection'}</h1>

  <div class="pair">
    <label class="grow">
      Host
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={host} required autofocus autocapitalize="none" autocorrect="off" spellcheck="false" placeholder="example.com" />
    </label>
    <label class="port">
      Port
      <input type="number" bind:value={port} required min="1" max="65535" inputmode="numeric" />
    </label>
  </div>

  <label>
    Username
    <input bind:value={username} required autocapitalize="none" autocorrect="off" spellcheck="false" />
  </label>

  <label>
    Name
    <input bind:value={name} placeholder="Optional — defaults to user@host" />
  </label>

  <div class="field">
    <span id="auth-kind-label">Sign in with</span>
    <Select
      bind:value={authKind}
      labelledby="auth-kind-label"
      options={[
        { value: 'key_file', label: 'Key file' },
        { value: 'password', label: 'Password' },
      ]}
    />
  </div>

  {#if authKind === 'key_file'}
    <label>
      Key file
      <input bind:value={keyPath} required autocapitalize="none" autocorrect="off" spellcheck="false" />
      <span class="help">A path on the machine running FastSSH. A passphrase is asked for when you connect.</span>
    </label>
  {:else}
    <p class="help">The password is asked for each time you connect; it is not stored.</p>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  {#if notice}<p class="help">{notice}</p>{/if}

  <div class="actions">
    <button type="submit" class="primary" disabled={busy}>Save</button>
    <button type="button" onclick={oncancel} disabled={busy}>Cancel</button>
    {#if initial}
      <span class="spacer"></span>
      {#if confirmingDelete}
        <button type="button" class="danger" onclick={remove} disabled={busy}>Delete permanently</button>
        <button type="button" onclick={() => (confirmingDelete = false)} disabled={busy}>Keep</button>
      {:else}
        <button type="button" onclick={forget} disabled={busy}>Forget host key</button>
        <button type="button" class="danger" onclick={() => (confirmingDelete = true)} disabled={busy}>Delete</button>
      {/if}
    {/if}
  </div>
</form>

<style>
  form {
    display: grid;
    gap: 16px;
  }

  h1 {
    font-size: 20px;
    font-weight: 650;
    margin: 0 0 4px;
  }

  .pair {
    display: flex;
    gap: 12px;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .port {
    width: 96px;
  }

  .help {
    color: var(--dim);
    font-size: 13px;
    margin: 0;
  }

  .error {
    color: var(--danger);
    margin: 0;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 4px;
  }

  .spacer {
    flex: 1;
  }
</style>
